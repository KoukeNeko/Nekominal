//! GPU glyph atlas: shelf (bucketed) packing with etagere, batched uploads from a
//! CPU shadow copy, growth by texture copy, wholesale clear when the size limit is hit.
//!
//! Instances store atlas coordinates in texels (not normalized UVs) and the
//! shader fetches with `textureLoad`, so growing the texture never invalidates
//! cached glyph positions and no sampler/filtering can blur glyph edges.
//!
//! Uploads cost far more per call than rasterizing a glyph, so inserts only write the CPU shadow and extend a
//! dirty rectangle; `flush` issues one `UpdateSubresource` per frame.

use etagere::{BucketedAtlasAllocator, size2};
use windows::Win32::Graphics::{
    Direct3D11::{ID3D11ShaderResourceView, ID3D11Texture2D},
    Dxgi::Common::DXGI_FORMAT,
};

use crate::d3d::{Gpu, atlas_texture, copy_texture, upload_texels};

pub struct AtlasTexture {
    pub texture: ID3D11Texture2D,
    pub view: ID3D11ShaderResourceView,
    alloc: BucketedAtlasAllocator,
    /// CPU mirror of the texture (row pitch = size * bytes_per_texel).
    shadow: Vec<u8>,
    /// Dirty bounding box in texels: (x0, y0, x1, y1), exclusive max.
    dirty: Option<(u32, u32, u32, u32)>,
    pub size: u32,
    max_size: u32,
    format: DXGI_FORMAT,
    bytes_per_texel: u32,
    /// Bumped whenever `texture` is replaced so bind groups can be rebuilt.
    pub generation: u32,
    pub clears: u32,
    pub grows: u32,
    pub flushes: u32,
}

#[derive(Debug)]
pub struct AtlasFull;

impl AtlasTexture {
    pub fn new(gpu: &Gpu, format: DXGI_FORMAT, bytes_per_texel: u32, initial: u32, max_size: u32) -> Self {
        let (texture, view) = atlas_texture(gpu, format, initial);
        AtlasTexture {
            texture,
            view,
            alloc: BucketedAtlasAllocator::new(size2(initial as i32, initial as i32)),
            shadow: vec![0; (initial * initial * bytes_per_texel) as usize],
            dirty: None,
            size: initial,
            max_size,
            format,
            bytes_per_texel,
            generation: 0,
            clears: 0,
            grows: 0,
            flushes: 0,
        }
    }

    /// GPU texture bytes (the CPU shadow doubles the host-side footprint).
    pub fn memory_bytes(&self) -> u64 {
        self.size as u64 * self.size as u64 * self.bytes_per_texel as u64
    }

    /// Fraction of texels currently allocated (shelf waste included).
    pub fn used_fraction(&self) -> f32 {
        self.alloc.allocated_space() as f32 / (self.size as f32 * self.size as f32)
    }

    /// Raise the size limit; the texture itself grows only when an insert needs the room.
    pub fn set_max_size(&mut self, max_size: u32) {
        self.max_size = self.max_size.max(max_size);
    }

    pub fn clear(&mut self) {
        self.alloc.clear();
        self.dirty = None;
        self.clears += 1;
    }

    /// Reserve `w x h` texels, growing the texture (up to `max_size`) if needed.
    fn allocate(&mut self, gpu: &Gpu, w: u32, h: u32) -> Result<(u32, u32), AtlasFull> {
        loop {
            if let Some(a) = self.alloc.allocate(size2(w as i32, h as i32)) {
                return Ok((a.rectangle.min.x as u32, a.rectangle.min.y as u32));
            }
            if self.size >= self.max_size {
                return Err(AtlasFull);
            }
            self.grow(gpu, (self.size * 2).min(self.max_size));
        }
    }

    fn grow(&mut self, gpu: &Gpu, new_size: u32) {
        // The old texture must be complete before the GPU-side copy.
        self.flush(gpu);
        let (texture, view) = atlas_texture(gpu, self.format, new_size);
        copy_texture(gpu, &texture, &self.texture);
        // Grow the shadow keeping existing rows (pitch changes).
        let bpt = self.bytes_per_texel as usize;
        let (old, new) = (self.size as usize, new_size as usize);
        let mut shadow = vec![0u8; new * new * bpt];
        for y in 0..old {
            shadow[y * new * bpt..y * new * bpt + old * bpt]
                .copy_from_slice(&self.shadow[y * old * bpt..(y + 1) * old * bpt]);
        }
        self.shadow = shadow;
        self.alloc.grow(size2(new_size as i32, new_size as i32));
        self.texture = texture;
        self.view = view;
        self.size = new_size;
        self.generation += 1;
        self.grows += 1;
    }

    /// Copy a bitmap into the shadow and mark it dirty. Returns its atlas position.
    pub fn insert(&mut self, gpu: &Gpu, w: u32, h: u32, data: &[u8]) -> Result<(u16, u16), AtlasFull> {
        debug_assert_eq!(data.len() as u32, w * h * self.bytes_per_texel);
        let (x, y) = self.allocate(gpu, w, h)?;
        let bpt = self.bytes_per_texel as usize;
        let pitch = self.size as usize * bpt;
        let row_bytes = w as usize * bpt;
        for row in 0..h as usize {
            let dst = (y as usize + row) * pitch + x as usize * bpt;
            self.shadow[dst..dst + row_bytes].copy_from_slice(&data[row * row_bytes..(row + 1) * row_bytes]);
        }
        self.dirty = Some(match self.dirty {
            None => (x, y, x + w, y + h),
            Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x + w), y1.max(y + h)),
        });
        Ok((x as u16, y as u16))
    }

    /// Upload everything inserted since the last flush with a single `UpdateSubresource`.
    pub fn flush(&mut self, gpu: &Gpu) {
        let Some(rect) = self.dirty.take() else { return };
        upload_texels(gpu, &self.texture, &self.shadow, self.size * self.bytes_per_texel, self.bytes_per_texel, rect);
        self.flushes += 1;
    }
}
