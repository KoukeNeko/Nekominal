//! GPU glyph atlas: shelf (bucketed) packing with etagere, batched uploads from a
//! CPU shadow copy, growth by texture copy, wholesale clear when the size limit is hit.
//!
//! Instances store atlas coordinates in texels (not normalized UVs) and the
//! shader fetches with `textureLoad`, so growing the texture never invalidates
//! cached glyph positions and no sampler/filtering can blur glyph edges.
//!
//! Uploads: measured `queue.write_texture` at ~28 us per call on DX12 versus ~4 us
//! to rasterize a glyph, so inserts only write the CPU shadow and extend a dirty
//! rectangle; `flush` issues one `write_texture` per frame.

use etagere::{BucketedAtlasAllocator, size2};

pub struct AtlasTexture {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    alloc: BucketedAtlasAllocator,
    /// CPU mirror of the texture (row pitch = size * bytes_per_texel).
    shadow: Vec<u8>,
    /// Dirty bounding box in texels: (x0, y0, x1, y1), exclusive max.
    dirty: Option<(u32, u32, u32, u32)>,
    pub size: u32,
    max_size: u32,
    format: wgpu::TextureFormat,
    bytes_per_texel: u32,
    label: &'static str,
    /// Bumped whenever `texture` is replaced so bind groups can be rebuilt.
    pub generation: u32,
    pub clears: u32,
    pub grows: u32,
    pub flushes: u32,
}

#[derive(Debug)]
pub struct AtlasFull;

impl AtlasTexture {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        bytes_per_texel: u32,
        initial: u32,
        max_size: u32,
        label: &'static str,
    ) -> Self {
        let (texture, view) = create_texture(device, format, initial, label);
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
            label,
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

    pub fn clear(&mut self) {
        self.alloc.clear();
        self.dirty = None;
        self.clears += 1;
    }

    /// Reserve `w x h` texels, growing the texture (up to `max_size`) if needed.
    fn allocate(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        w: u32,
        h: u32,
    ) -> Result<(u32, u32), AtlasFull> {
        loop {
            if let Some(a) = self.alloc.allocate(size2(w as i32, h as i32)) {
                return Ok((a.rectangle.min.x as u32, a.rectangle.min.y as u32));
            }
            if self.size >= self.max_size {
                return Err(AtlasFull);
            }
            self.grow(device, queue, (self.size * 2).min(self.max_size));
        }
    }

    fn grow(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, new_size: u32) {
        // The old texture must be complete before the GPU-side copy.
        self.flush(queue);
        let (texture, view) = create_texture(device, self.format, new_size, self.label);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("atlas grow") });
        encoder.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::Extent3d { width: self.size, height: self.size, depth_or_array_layers: 1 },
        );
        queue.submit([encoder.finish()]);
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
    pub fn insert(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        w: u32,
        h: u32,
        data: &[u8],
    ) -> Result<(u16, u16), AtlasFull> {
        debug_assert_eq!(data.len() as u32, w * h * self.bytes_per_texel);
        let (x, y) = self.allocate(device, queue, w, h)?;
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

    /// Upload everything inserted since the last flush with a single `write_texture`.
    pub fn flush(&mut self, queue: &wgpu::Queue) {
        let Some((x0, y0, x1, y1)) = self.dirty.take() else { return };
        let bpt = self.bytes_per_texel;
        let pitch = self.size * bpt;
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x: x0, y: y0, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            &self.shadow,
            wgpu::TexelCopyBufferLayout {
                offset: (y0 * pitch + x0 * bpt) as u64,
                bytes_per_row: Some(pitch),
                rows_per_image: Some(y1 - y0),
            },
            wgpu::Extent3d { width: x1 - x0, height: y1 - y0, depth_or_array_layers: 1 },
        );
        self.flushes += 1;
    }
}

fn create_texture(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    size: u32,
    label: &'static str,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: size, height: size, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}
