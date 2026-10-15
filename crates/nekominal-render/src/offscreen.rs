//! Headless GPU context, offscreen render target, readback and PNG output.

use std::path::Path;

use windows::Win32::Graphics::{
    Direct3D11::*,
    Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC},
};

pub use crate::d3d::{AdapterKind, Gpu};

pub fn create_gpu(kind: AdapterKind) -> Result<Gpu, String> {
    Gpu::new(kind)
}

/// A BGRA8 texture to render into and read back.
pub struct OffscreenTarget {
    texture: ID3D11Texture2D,
    pub view: ID3D11RenderTargetView,
    staging: ID3D11Texture2D,
    pub width: u32,
    pub height: u32,
}

impl OffscreenTarget {
    pub fn new(gpu: &Gpu, width: u32, height: u32) -> Self {
        let desc = |usage, bind: D3D11_BIND_FLAG, cpu: D3D11_CPU_ACCESS_FLAG| D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: usage,
            BindFlags: bind.0 as u32,
            CPUAccessFlags: cpu.0 as u32,
            ..Default::default()
        };
        let make = |d: D3D11_TEXTURE2D_DESC| {
            let mut texture = None;
            // SAFETY: the descriptor lives through the call.
            unsafe { gpu.device.CreateTexture2D(&d, None, Some(&mut texture)) }.expect("CreateTexture2D");
            texture.expect("CreateTexture2D returned no texture")
        };
        let texture = make(desc(D3D11_USAGE_DEFAULT, D3D11_BIND_RENDER_TARGET, D3D11_CPU_ACCESS_FLAG(0)));
        let staging = make(desc(D3D11_USAGE_STAGING, D3D11_BIND_FLAG(0), D3D11_CPU_ACCESS_READ));
        let mut view = None;
        // SAFETY: the texture lives through the call.
        unsafe { gpu.device.CreateRenderTargetView(&texture, None, Some(&mut view)) }.expect("CreateRenderTargetView");
        OffscreenTarget { texture, view: view.expect("no render target view"), staging, width, height }
    }

    /// Waits for the drawing and returns tightly packed RGBA8 (converted from BGRA).
    pub fn read_rgba(&self, gpu: &Gpu) -> Result<Vec<u8>, String> {
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        // SAFETY: the mapped pointer is valid for `RowPitch * height` bytes until `Unmap`.
        unsafe {
            gpu.context.CopyResource(&self.staging, &self.texture);
            gpu.context.Map(&self.staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped)).map_err(|e| format!("Map: {e}"))?;
            let mut out = Vec::with_capacity((self.width * self.height * 4) as usize);
            for y in 0..self.height as usize {
                let row = std::slice::from_raw_parts(mapped.pData.cast::<u8>().add(y * mapped.RowPitch as usize), self.width as usize * 4);
                out.extend_from_slice(row);
            }
            gpu.context.Unmap(&self.staging, 0);
            for px in out.as_chunks_mut::<4>().0 {
                px.swap(0, 2);
            }
            Ok(out)
        }
    }
}

pub fn write_png(path: &Path, width: u32, height: u32, rgba: &[u8]) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().map_err(|e| e.to_string())?;
    w.write_image_data(rgba).map_err(|e| e.to_string())
}
