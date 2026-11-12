//! GPU-side types: instance layouts, uniforms, pipelines, growable buffers.

use bytemuck::{Pod, Zeroable};

use crate::gfx::{Buffer, DrawStates, Gpu, Program, dynamic_buffer, write_buffer};
use crate::shaders;

/// What a field of an instance record is made of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Float32x4,
    Float32x2,
    Sint32x2,
    Uint16x2,
    /// Four bytes as 0..=1 floats.
    Unorm8x4,
    Uint32,
}

/// One field of an instance record: its format and byte offset. The fields are the inputs of the vertex shader, in order.
pub struct Attribute(pub Format, pub u32);

/// What a buffer is bound as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BufferKind {
    /// Instance records.
    Vertex,
    Constant,
}

/// The texel format of an atlas.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TexelFormat {
    R8,
    Rgba8,
}

impl TexelFormat {
    pub const fn bytes(self) -> u32 {
        match self {
            TexelFormat::R8 => 1,
            TexelFormat::Rgba8 => 4,
        }
    }
}

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
    pub buffer: Buffer,
    pub stride: u32,
    capacity: u64,
    pub count: u32,
}

impl GrowBuffer {
    pub fn new(gpu: &Gpu, stride: u32, capacity: u64) -> Result<Self, String> {
        let capacity = capacity.max(256).next_power_of_two();
        Ok(GrowBuffer { buffer: dynamic_buffer(gpu, BufferKind::Vertex, capacity)?, stride, capacity, count: 0 })
    }

    pub fn write(&mut self, gpu: &Gpu, bytes: &[u8], count: u32) {
        self.count = count;
        if bytes.is_empty() {
            return;
        }
        if bytes.len() as u64 > self.capacity {
            let capacity = (bytes.len() as u64).next_power_of_two();
            match dynamic_buffer(gpu, BufferKind::Vertex, capacity) {
                Ok(buffer) => (self.buffer, self.capacity) = (buffer, capacity),
                // The device is gone (it is made again then): draw what the old buffer holds, not past it.
                Err(_) => {
                    self.count = (self.capacity / u64::from(self.stride.max(1))) as u32;
                    write_buffer(gpu, &self.buffer, &bytes[..(u64::from(self.count) * u64::from(self.stride)) as usize]);
                    return;
                }
            }
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

pub const RECT_ATTRS: [Attribute; 2] = [Attribute(Format::Float32x4, 0), Attribute(Format::Unorm8x4, 16)];
pub const GLYPH_ATTRS: [Attribute; 5] = [
    Attribute(Format::Sint32x2, 0),
    Attribute(Format::Uint16x2, 8),
    Attribute(Format::Uint16x2, 12),
    Attribute(Format::Unorm8x4, 16),
    Attribute(Format::Uint32, 20),
];

/// Shaders and blend state of the renderer.
pub struct Pipelines {
    pub states: DrawStates,
    pub rect: Program,
    pub glyph: Program,
    /// Per-channel coverage text; the plain glyph program where the device cannot blend that way.
    pub glyph_lcd: Program,
}

impl Pipelines {
    pub fn new(gpu: &Gpu) -> Result<Self, String> {
        let stride = std::mem::size_of::<GlyphInstance>() as u32;
        Ok(Pipelines {
            states: DrawStates::new(gpu)?,
            rect: Program::new(gpu, &shaders::RECT, &RECT_ATTRS, std::mem::size_of::<RectInstance>() as u32)?,
            glyph: Program::new(gpu, &shaders::GLYPH, &GLYPH_ATTRS, stride)?,
            glyph_lcd: Program::new(gpu, if gpu.dual_source_supported() { &shaders::GLYPH_LCD } else { &shaders::GLYPH }, &GLYPH_ATTRS, stride)?,
        })
    }
}
