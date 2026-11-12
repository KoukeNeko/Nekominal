//! Pictures over the grid (Sixel, kitty graphics): the plain data the view hands in (`ImageView`), one texture per
//! picture kept while it is in view, and a textured-quad pipeline drawn in three layers by the picture's z index, as
//! kitty layers them: below the cell backgrounds (z < -1,073,741,824), between the backgrounds and the text (z < 0),
//! and over the text. Quads are clipped to the pane by its viewport and scissor like everything else.

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use crate::cell::GridView;
use crate::gfx::{Gpu, Program, TextureView, bind_picture, picture_texture, set_program};
use crate::gpu::{Attribute, Format, GrowBuffer};
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

const IMAGE_ATTRS: [Attribute; 2] = [Attribute(Format::Float32x4, 0), Attribute(Format::Float32x4, 16)];

pub(crate) struct ImagePipeline {
    program: Program,
}

impl ImagePipeline {
    fn new(gpu: &Gpu) -> Result<Self, String> {
        Ok(ImagePipeline { program: Program::new(gpu, &crate::shaders::IMAGE, &IMAGE_ATTRS, std::mem::size_of::<ImageInstance>() as u32)? })
    }
}

/// A picture's pixels on the GPU, premultiplied, so bilinear filtering does not darken its transparent edges.
fn upload(gpu: &Gpu, image: &ImageView) -> Option<TextureView> {
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
    picture_texture(gpu, width, height, &premultiplied)
}

/// The pictures of one pane on the GPU.
#[derive(Default)]
pub(crate) struct PaneImages {
    /// Textures by picture key; one whose key leaves the view is released.
    textures: Vec<(u64, TextureView)>,
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
                set_program(&self.gpu, &pipeline.program);
                drew = true;
            }
            bind_picture(&self.gpu, texture);
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

    #[cfg(windows)]
    #[test]
    fn the_picture_shader_compiles() {
        let source = include_str!("shaders/image.hlsl");
        for (entry, target) in [("vs_main", "vs_5_0"), ("fs_main", "ps_5_0")] {
            let code = crate::gfx::compile(source, entry, target).unwrap_or_else(|e| panic!("{e}"));
            assert!(code.starts_with(b"DXBC"));
        }
    }
}
