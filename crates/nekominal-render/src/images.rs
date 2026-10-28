//! Pictures over the grid (Sixel, kitty graphics): the plain data the view hands in (`ImageView`), one texture per
//! picture kept while it is in view, and a textured-quad pipeline drawn in three layers by the picture's z index, as
//! kitty layers them: below the cell backgrounds (z < -1,073,741,824), between the backgrounds and the text (z < 0),
//! and over the text. Quads are clipped to the pane by its viewport and scissor like everything else.

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use windows::Win32::Graphics::{
    Direct3D11::*,
    Dxgi::Common::{DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_FORMAT_R32G32B32A32_FLOAT, DXGI_SAMPLE_DESC},
};

use crate::cell::GridView;
use crate::d3d::{Attribute, Gpu, input_layout, pixel_shader, set_shaders, vertex_shader};
use crate::gpu::GrowBuffer;
use crate::renderer::{PaneState, Renderer};

/// A picture placed on the grid.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageView {
    /// Identity of the pixels: one texture is kept per key while the key is in view.
    pub key: u64,
    pub width: u32,
    pub height: u32,
    /// RGBA, straight alpha, `width * height * 4` bytes.
    pub pixels: Arc<[u8]>,
    /// The part drawn: x, y, width, height in picture pixels.
    pub src: [u32; 4],
    /// The cell of its top-left corner (the row is negative when it starts above the view) and the pixels into it.
    pub row: i32,
    pub col: i32,
    pub offset: [u32; 2],
    /// Cells it is scaled into; None draws `src` at its own size.
    pub cells: Option<[u32; 2]>,
    pub z: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Layer {
    UnderBackgrounds,
    UnderText,
    OverText,
}

impl Layer {
    fn of(z: i32) -> Layer {
        match z {
            ..-1_073_741_824 => Layer::UnderBackgrounds,
            -1_073_741_824..0 => Layer::UnderText,
            _ => Layer::OverText,
        }
    }
}

/// One quad. 32 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
struct ImageInstance {
    /// x, y, width, height in pane pixels.
    dst: [f32; 4],
    /// u, v, width, height in texture coordinates.
    src: [f32; 4],
}

const IMAGE_ATTRS: [Attribute; 2] =
    [Attribute(DXGI_FORMAT_R32G32B32A32_FLOAT, 0), Attribute(DXGI_FORMAT_R32G32B32A32_FLOAT, 16)];

pub(crate) struct ImagePipeline {
    layout: ID3D11InputLayout,
    vs: ID3D11VertexShader,
    ps: ID3D11PixelShader,
    sampler: ID3D11SamplerState,
}

impl ImagePipeline {
    fn new(gpu: &Gpu) -> Result<Self, String> {
        use crate::shaders::{IMAGE_PS, IMAGE_VS};
        let desc = D3D11_SAMPLER_DESC {
            Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
            AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
            AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
            AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
            MaxLOD: f32::MAX,
            ..Default::default()
        };
        let mut sampler = None;
        // SAFETY: the descriptor lives through the call.
        unsafe { gpu.device.CreateSamplerState(&desc, Some(&mut sampler)) }.map_err(|e| format!("CreateSamplerState: {e}"))?;
        Ok(ImagePipeline {
            layout: input_layout(gpu, IMAGE_VS, &IMAGE_ATTRS)?,
            vs: vertex_shader(gpu, IMAGE_VS)?,
            ps: pixel_shader(gpu, IMAGE_PS)?,
            sampler: sampler.ok_or("no sampler")?,
        })
    }
}

/// A picture's pixels on the GPU, premultiplied, so bilinear filtering does not darken its transparent edges.
fn upload(gpu: &Gpu, image: &ImageView) -> Option<ID3D11ShaderResourceView> {
    let (width, height) = (image.width, image.height);
    if width == 0 || height == 0 || image.pixels.len() < (width * height * 4) as usize {
        return None;
    }
    let premultiplied: Vec<u8> = image.pixels[..(width * height * 4) as usize]
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|&[r, g, b, a]| {
            let scale = |c: u8| ((u16::from(c) * u16::from(a) + 127) / 255) as u8;
            [scale(r), scale(g), scale(b), a]
        })
        .collect();
    let desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_R8G8B8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: D3D11_USAGE_IMMUTABLE,
        BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
        ..Default::default()
    };
    let data = D3D11_SUBRESOURCE_DATA { pSysMem: premultiplied.as_ptr().cast(), SysMemPitch: width * 4, SysMemSlicePitch: 0 };
    let mut texture = None;
    let mut view = None;
    // SAFETY: the descriptor and the pixels live through the calls; the view keeps the texture alive.
    unsafe {
        gpu.device.CreateTexture2D(&desc, Some(&data), Some(&mut texture)).ok()?;
        gpu.device.CreateShaderResourceView(&texture?, None, Some(&mut view)).ok()?;
    }
    view
}

/// The pictures of one pane on the GPU.
#[derive(Default)]
pub(crate) struct PaneImages {
    /// Textures by picture key; one whose key leaves the view is released.
    textures: Vec<(u64, ID3D11ShaderResourceView)>,
    /// The quads in drawing order (by z, then oldest first) with their layer and texture.
    quads: Vec<(Layer, u64, ImageInstance)>,
    previous: Vec<(Layer, u64, ImageInstance)>,
    buffer: Option<GrowBuffer>,
    gpu_gen: u64,
}

impl Renderer {
    /// Brings the pane's pictures up to date with `view`; true when what they draw changed.
    pub(crate) fn update_images(&mut self, pane: &mut PaneState, view: &GridView<'_>) -> bool {
        let images = &mut pane.images;
        if images.gpu_gen != self.gpu_gen {
            // A new device: what was made on the old one is gone.
            *images = PaneImages { gpu_gen: self.gpu_gen, ..PaneImages::default() };
        }
        if view.images.is_empty() && images.textures.is_empty() && images.previous.is_empty() {
            return false;
        }
        if self.image_pipeline.is_none() {
            self.image_pipeline = ImagePipeline::new(&self.gpu).ok();
        }
        let (cw, ch) = (self.metrics.width as f32, self.metrics.height as f32);
        let mut order: Vec<&ImageView> = view.images.iter().collect();
        order.sort_by_key(|image| image.z);
        images.quads.clear();
        for image in order {
            let [sx, sy, sw, sh] = image.src;
            let (w, h) = match image.cells {
                Some([columns, rows]) => (columns as f32 * cw, rows as f32 * ch),
                None => (sw as f32, sh as f32),
            };
            if sw == 0 || sh == 0 || w <= 0.0 || h <= 0.0 || image.width == 0 || image.height == 0 {
                continue;
            }
            if !images.textures.iter().any(|(key, _)| *key == image.key) {
                let Some(texture) = upload(&self.gpu, image) else { continue };
                images.textures.push((image.key, texture));
            }
            let (iw, ih) = (image.width as f32, image.height as f32);
            let instance = ImageInstance {
                dst: [image.col as f32 * cw + image.offset[0] as f32, image.row as f32 * ch + image.offset[1] as f32, w, h],
                src: [sx as f32 / iw, sy as f32 / ih, sw as f32 / iw, sh as f32 / ih],
            };
            images.quads.push((Layer::of(image.z), image.key, instance));
        }
        images.textures.retain(|(key, _)| view.images.iter().any(|image| image.key == *key));
        if images.quads == images.previous {
            return false;
        }
        let instances: Vec<ImageInstance> = images.quads.iter().map(|(_, _, instance)| *instance).collect();
        if !instances.is_empty() {
            if images.buffer.is_none() {
                images.buffer = GrowBuffer::new(&self.gpu, std::mem::size_of::<ImageInstance>() as u32, 1024).ok();
            }
            if let Some(buffer) = &mut images.buffer {
                buffer.write(&self.gpu, bytemuck::cast_slice(&instances), instances.len() as u32);
            }
        }
        images.previous.clone_from(&images.quads);
        true
    }

    /// Draws the pane's pictures of `layer`; true when it drew any (the shaders of the caller must be set again).
    pub(crate) fn draw_images(&self, pane: &PaneState, layer: Layer) -> bool {
        let images = &pane.images;
        let (Some(pipeline), Some(buffer)) = (&self.image_pipeline, &images.buffer) else { return false };
        let mut drew = false;
        for (index, (on, key, _)) in images.quads.iter().enumerate() {
            if *on != layer || index as u32 >= buffer.count {
                continue;
            }
            let Some((_, texture)) = images.textures.iter().find(|(k, _)| k == key) else { continue };
            if !drew {
                set_shaders(&self.gpu, &pipeline.layout, &pipeline.vs, &pipeline.ps);
                // SAFETY: the sampler lives through the call.
                unsafe { self.gpu.context.PSSetSamplers(0, Some(&[Some(pipeline.sampler.clone())])) };
                drew = true;
            }
            // SAFETY: the view lives through the call.
            unsafe { self.gpu.context.PSSetShaderResources(2, Some(&[Some(texture.clone())])) };
            self.gpu.draw_instances_from(&buffer.buffer, buffer.stride, index as u32..index as u32 + 1);
        }
        drew
    }
}

#[cfg(test)]
mod tests {
    use super::Layer;

    #[test]
    fn z_picks_the_layer_as_kitty_does() {
        assert_eq!(Layer::of(i32::MIN), Layer::UnderBackgrounds);
        assert_eq!(Layer::of(-1_073_741_825), Layer::UnderBackgrounds);
        assert_eq!(Layer::of(-1_073_741_824), Layer::UnderText);
        assert_eq!(Layer::of(-1), Layer::UnderText);
        assert_eq!(Layer::of(0), Layer::OverText);
    }

    #[test]
    fn the_picture_shader_compiles() {
        let source = include_str!("shaders/image.hlsl");
        for (entry, target) in [("vs_main", "vs_5_0"), ("fs_main", "ps_5_0")] {
            let code = crate::d3d::compile(source, entry, target).unwrap_or_else(|e| panic!("{e}"));
            assert!(code.starts_with(b"DXBC"));
        }
    }
}
