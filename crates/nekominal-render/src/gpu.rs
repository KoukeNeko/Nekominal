//! GPU-side types: instance layouts, uniforms, pipelines, growable buffers.

use bytemuck::{Pod, Zeroable};
use windows::Win32::Graphics::{Direct3D11::*, Dxgi::Common::*};

use crate::d3d::{Attribute, DrawStates, Gpu, compile, dynamic_buffer, input_layout, pixel_shader, vertex_shader, write_buffer};

/// One solid rectangle in pane pixels. 20 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct RectInstance {
    /// x, y, width, height in pixels.
    pub rect: [f32; 4],
    /// Straight (non-premultiplied) RGBA8.
    pub color: [u8; 4],
}

/// One glyph bitmap placed 1:1 on the pixel grid. 24 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct GlyphInstance {
    /// Top-left in pane pixels (may be negative for overhang).
    pub dst: [i32; 2],
    /// Bitmap size in texels (== pixels drawn).
    pub size: [u16; 2],
    /// Top-left in the atlas, texels.
    pub src: [u16; 2],
    pub fg: [u8; 4],
    /// 0 = R8 mask, 1 = RGBA LCD mask, 2 = RGBA premultiplied color.
    pub kind: u32,
}

pub const KIND_MASK: u32 = 0;
pub const KIND_LCD: u32 = 1;
pub const KIND_COLOR: u32 = 2;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct Uniforms {
    pub viewport: [f32; 2],
    /// x: grayscale enhanced contrast, y: LCD enhanced contrast.
    pub contrast: [f32; 2],
    pub gamma_ratios: [f32; 4],
    /// bit 0: apply contrast + gamma correction to text coverage.
    pub flags: u32,
    /// Extra enhanced contrast for grayscale text, added for every foreground colour. Non-zero only while
    /// the pane background is translucent (see `RendererConfig::translucent_text_boost`).
    pub text_boost: f32,
    pub pad: [u32; 2],
}

/// DirectWrite-style gamma ratios (Windows Terminal `atlas/dwrite.cpp`, MIT).
/// `gamma` is clamped to 1.0..=2.2 in 0.1 steps; DirectWrite's default is 1.8.
pub fn gamma_ratios(gamma: f32) -> [f32; 4] {
    #[rustfmt::skip]
    const TABLE: [[f32; 4]; 13] = [
        [0.0000 / 4.0, 0.0000 / 4.0, 0.0000 / 4.0, 0.0000 / 4.0],
        [0.0166 / 4.0, -0.0807 / 4.0, 0.2227 / 4.0, -0.0751 / 4.0],
        [0.0350 / 4.0, -0.1760 / 4.0, 0.4325 / 4.0, -0.1370 / 4.0],
        [0.0543 / 4.0, -0.2821 / 4.0, 0.6302 / 4.0, -0.1876 / 4.0],
        [0.0739 / 4.0, -0.3963 / 4.0, 0.8167 / 4.0, -0.2287 / 4.0],
        [0.0933 / 4.0, -0.5161 / 4.0, 0.9926 / 4.0, -0.2616 / 4.0],
        [0.1121 / 4.0, -0.6395 / 4.0, 1.1588 / 4.0, -0.2877 / 4.0],
        [0.1300 / 4.0, -0.7649 / 4.0, 1.3159 / 4.0, -0.3080 / 4.0],
        [0.1469 / 4.0, -0.8911 / 4.0, 1.4644 / 4.0, -0.3234 / 4.0],
        [0.1627 / 4.0, -1.0170 / 4.0, 1.6051 / 4.0, -0.3347 / 4.0],
        [0.1773 / 4.0, -1.1420 / 4.0, 1.7385 / 4.0, -0.3426 / 4.0],
        [0.1908 / 4.0, -1.2652 / 4.0, 1.8650 / 4.0, -0.3476 / 4.0],
        [0.2031 / 4.0, -1.3864 / 4.0, 1.9851 / 4.0, -0.3501 / 4.0],
    ];
    const NORM13: f32 = ((0x10000 as f64) / (255.0 * 255.0) * 4.0) as f32;
    const NORM24: f32 = ((0x100 as f64) / 255.0 * 4.0) as f32;
    let index = ((gamma * 10.0).round() as usize).clamp(10, 22) - 10;
    let r = TABLE[index];
    [r[0] * NORM13, r[1] * NORM24, r[2] * NORM13, r[3] * NORM24]
}


/// Instance buffer that is rewritten wholesale each time and only reallocated when the payload outgrows it
/// (power-of-two growth).
pub struct GrowBuffer {
    pub buffer: ID3D11Buffer,
    pub stride: u32,
    capacity: u64,
    pub count: u32,
}

impl GrowBuffer {
    pub fn new(gpu: &Gpu, stride: u32, capacity: u64) -> Self {
        let capacity = capacity.max(256).next_power_of_two();
        GrowBuffer { buffer: dynamic_buffer(gpu, D3D11_BIND_VERTEX_BUFFER, capacity), stride, capacity, count: 0 }
    }

    pub fn write(&mut self, gpu: &Gpu, bytes: &[u8], count: u32) {
        self.count = count;
        if bytes.is_empty() {
            return;
        }
        if bytes.len() as u64 > self.capacity {
            self.capacity = (bytes.len() as u64).next_power_of_two();
            self.buffer = dynamic_buffer(gpu, D3D11_BIND_VERTEX_BUFFER, self.capacity);
        }
        write_buffer(gpu, &self.buffer, bytes);
    }

    pub fn capacity(&self) -> u64 {
        self.capacity
    }

    /// Draws every instance written.
    pub fn draw(&self, gpu: &Gpu) {
        if self.count > 0 {
            gpu.draw_instances(&self.buffer, self.stride, self.count);
        }
    }
}

pub const RECT_ATTRS: [Attribute; 2] = [Attribute(DXGI_FORMAT_R32G32B32A32_FLOAT, 0), Attribute(DXGI_FORMAT_R8G8B8A8_UNORM, 16)];
pub const GLYPH_ATTRS: [Attribute; 5] = [
    Attribute(DXGI_FORMAT_R32G32_SINT, 0),
    Attribute(DXGI_FORMAT_R16G16_UINT, 8),
    Attribute(DXGI_FORMAT_R16G16_UINT, 12),
    Attribute(DXGI_FORMAT_R8G8B8A8_UNORM, 16),
    Attribute(DXGI_FORMAT_R32_UINT, 20),
];

/// Shaders, input layouts and blend state of the renderer.
pub struct Pipelines {
    pub states: DrawStates,
    pub rect_layout: ID3D11InputLayout,
    pub rect_vs: ID3D11VertexShader,
    pub rect_ps: ID3D11PixelShader,
    pub glyph_layout: ID3D11InputLayout,
    pub glyph_vs: ID3D11VertexShader,
    pub glyph_ps: ID3D11PixelShader,
    pub glyph_lcd_ps: ID3D11PixelShader,
}

impl Pipelines {
    pub fn new(gpu: &Gpu) -> Result<Self, String> {
        let rect_src = include_str!("shaders/rect.hlsl");
        let glyph_src = include_str!("shaders/glyph.hlsl");
        let rect_vs_code = compile(rect_src, "vs_main", "vs_5_0")?;
        let glyph_vs_code = compile(glyph_src, "vs_main", "vs_5_0")?;
        Ok(Pipelines {
            states: DrawStates::new(gpu)?,
            rect_layout: input_layout(gpu, &rect_vs_code, &RECT_ATTRS)?,
            rect_vs: vertex_shader(gpu, &rect_vs_code)?,
            rect_ps: pixel_shader(gpu, &compile(rect_src, "fs_main", "ps_5_0")?)?,
            glyph_layout: input_layout(gpu, &glyph_vs_code, &GLYPH_ATTRS)?,
            glyph_vs: vertex_shader(gpu, &glyph_vs_code)?,
            glyph_ps: pixel_shader(gpu, &compile(glyph_src, "fs_main", "ps_5_0")?)?,
            glyph_lcd_ps: pixel_shader(gpu, &compile(glyph_src, "fs_lcd", "ps_5_0")?)?,
        })
    }
}