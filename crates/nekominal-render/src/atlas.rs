//! GPU glyph atlas: shelf (bucketed) packing with etagere, the glyphs of a frame uploaded together, growth by texture
//! copy, wholesale clear when the size limit is hit.
//!
//! Instances store atlas coordinates in texels (not normalized UVs) and the
//! shader fetches with `textureLoad`, so growing the texture never invalidates
//! cached glyph positions and no sampler/filtering can blur glyph edges.
//!
//! Inserts keep only the glyph's own pixels until `flush`, which uploads each of them into its place: no copy of the
//! whole texture is kept in memory (it cost as much as the texture, per pane, up to 20 MiB once grown), and most
//! frames insert nothing.

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
    /// The glyphs inserted since the last flush: their box in texels (x0, y0, x1, y1, exclusive max) and their pixels.
    pending: Vec<((u32, u32, u32, u32), Vec<u8>)>,
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
    pub fn new(gpu: &Gpu, format: DXGI_FORMAT, bytes_per_texel: u32, initial: u32, max_size: u32) -> Result<Self, String> {
        let (texture, view) = atlas_texture(gpu, format, initial)?;
        Ok(AtlasTexture {
            texture,
            view,
            alloc: BucketedAtlasAllocator::new(size2(initial as i32, initial as i32)),
            pending: Vec::new(),
            size: initial,
            max_size,
            format,
            bytes_per_texel,
            generation: 0,
            clears: 0,
            grows: 0,
            flushes: 0,
        })
    }

    /// GPU texture bytes.
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
        self.pending.clear();
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
            // A texture the device cannot make (it was removed) is as good as no room.
            self.grow(gpu, (self.size * 2).min(self.max_size)).map_err(|_| AtlasFull)?;
        }
    }

    fn grow(&mut self, gpu: &Gpu, new_size: u32) -> Result<(), String> {
        // The old texture must be complete before the GPU-side copy.
        self.flush(gpu);
        let (texture, view) = atlas_texture(gpu, self.format, new_size)?;
        copy_texture(gpu, &texture, &self.texture);
        self.alloc.grow(size2(new_size as i32, new_size as i32));
        self.texture = texture;
        self.view = view;
        self.size = new_size;
        self.generation += 1;
        self.grows += 1;
        Ok(())
    }

    /// Keeps a bitmap to be uploaded at the next flush. Returns its atlas position.
    pub fn insert(&mut self, gpu: &Gpu, w: u32, h: u32, data: &[u8]) -> Result<(u16, u16), AtlasFull> {
        debug_assert_eq!(data.len() as u32, w * h * self.bytes_per_texel);
        let (x, y) = self.allocate(gpu, w, h)?;
        if w > 0 && h > 0 {
            self.pending.push(((x, y, x + w, y + h), data.to_vec()));
        }
        Ok((x as u16, y as u16))
    }

    /// Uploads every glyph inserted since the last flush into its place.
    pub fn flush(&mut self, gpu: &Gpu) {
        if self.pending.is_empty() {
            return;
        }
        for (rect, pixels) in self.pending.drain(..) {
            upload_texels(gpu, &self.texture, &pixels, (rect.2 - rect.0) * self.bytes_per_texel, rect);
        }
        self.flushes += 1;
    }
}
