//! The Direct3D 11 side of the renderer (`gfx` on Windows): the device and its immediate context, and the small helpers
//! (buffers, textures, programs, draw state) the renderer and the UI layer are built from. One `Gpu` is used from one
//! thread. The wgpu side (`wgpu_gfx.rs`) offers the same names, so the code above this layer is the same on both.

use std::ffi::c_void;

use crate::gpu::{Attribute, BufferKind, Format, TexelFormat};

use windows::{
    Win32::{
        Foundation::{HMODULE, RECT},
        Graphics::{
            Direct3D::{
                D3D_DRIVER_TYPE_UNKNOWN, D3D_DRIVER_TYPE_WARP, D3D_FEATURE_LEVEL_11_0, D3D_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP,
                Fxc::D3DCompile,
            },
            Direct3D11::*,
            Dxgi::{Common::*, *},
        },
    },
    core::{Interface, PCSTR},
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AdapterKind {
    /// The low-power hardware adapter (the integrated GPU where there are two).
    Real,
    /// The adapter that drives the monitor (an HMONITOR as a number), so that what is drawn does not have to be copied from
    /// one graphics card to another to reach the screen; the low-power one when no adapter has the monitor.
    Display(isize),
    /// Microsoft Basic Render Driver (software rasterizer).
    Warp,
}

/// The adapter that has an output on the monitor, None when no hardware adapter does.
fn adapter_of_monitor(factory: &IDXGIFactory6, monitor: isize) -> Option<IDXGIAdapter1> {
    // SAFETY: plain DXGI queries; the objects are released when they go out of scope.
    unsafe {
        let mut adapters = 0;
        while let Ok(adapter) = factory.EnumAdapters1(adapters) {
            let mut outputs = 0;
            while let Ok(output) = adapter.EnumOutputs(outputs) {
                if output.GetDesc().is_ok_and(|desc| desc.Monitor.0 as isize == monitor) {
                    return Some(adapter);
                }
                outputs += 1;
            }
            adapters += 1;
        }
    }
    None
}

/// What a pass draws into.
pub type RenderTarget = ID3D11RenderTargetView;
pub type Buffer = ID3D11Buffer;
pub type Texture = ID3D11Texture2D;
/// A texture as the shaders read it.
pub type TextureView = ID3D11ShaderResourceView;

#[derive(Clone)]
pub struct Gpu {
    pub device: ID3D11Device,
    pub context: ID3D11DeviceContext,
    pub adapter_name: String,
    /// Bilinear, clamped: how a picture over the grid is scaled.
    picture_sampler: ID3D11SamplerState,
}

impl Gpu {
    pub fn new(kind: AdapterKind) -> Result<Gpu, String> {
        // SAFETY: plain DXGI/D3D11 creation calls; every pointer outlives its call.
        unsafe {
            let mut adapter_name = String::from("WARP");
            let (adapter, driver) = match kind {
                AdapterKind::Warp => (None, D3D_DRIVER_TYPE_WARP),
                AdapterKind::Real | AdapterKind::Display(_) => {
                    let factory: IDXGIFactory6 = CreateDXGIFactory1().map_err(|e| format!("CreateDXGIFactory1: {e}"))?;
                    let driving = match kind {
                        AdapterKind::Display(monitor) => adapter_of_monitor(&factory, monitor),
                        _ => None,
                    };
                    let adapter: IDXGIAdapter1 = match driving {
                        Some(adapter) => adapter,
                        None => factory
                            .EnumAdapterByGpuPreference(0, DXGI_GPU_PREFERENCE_MINIMUM_POWER)
                            .map_err(|e| format!("no hardware adapter: {e}"))?,
                    };
                    let desc = adapter.GetDesc1().map_err(|e| format!("GetDesc1: {e}"))?;
                    adapter_name = String::from_utf16_lossy(&desc.Description).trim_end_matches('\0').to_owned();
                    (Some(adapter.cast::<IDXGIAdapter>().map_err(|e| e.to_string())?), D3D_DRIVER_TYPE_UNKNOWN)
                }
            };
            let mut device = None;
            let mut context = None;
            D3D11CreateDevice(
                adapter.as_ref(),
                driver,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&[D3D_FEATURE_LEVEL_11_0]),
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
            .map_err(|e| format!("D3D11CreateDevice: {e}"))?;
            let device: ID3D11Device = device.ok_or("no device")?;
            let desc = D3D11_SAMPLER_DESC {
                Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
                AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
                AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
                AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
                MaxLOD: f32::MAX,
                ..Default::default()
            };
            let mut picture_sampler = None;
            device.CreateSamplerState(&desc, Some(&mut picture_sampler)).map_err(|e| format!("CreateSamplerState: {e}"))?;
            Ok(Gpu { device, context: context.ok_or("no context")?, adapter_name, picture_sampler: picture_sampler.ok_or("no sampler")? })
        }
    }

    /// Dual-source blending (LCD text) is part of feature level 11.
    pub fn dual_source_supported(&self) -> bool {
        true
    }

    /// Hands the commands recorded so far to the GPU. Direct3D 11 runs them as they come.
    pub fn flush(&self) {}

    /// Makes `target` the render target, with the viewport and the scissor rectangle both set to `rect`
    /// (x, y, width, height in pixels).
    pub fn bind_target(&self, target: &RenderTarget, rect: [u32; 4]) {
        // SAFETY: the views and states live through the calls.
        unsafe {
            self.context.OMSetRenderTargets(Some(&[Some(target.clone())]), None);
            self.context.RSSetViewports(Some(&[D3D11_VIEWPORT {
                TopLeftX: rect[0] as f32,
                TopLeftY: rect[1] as f32,
                Width: rect[2] as f32,
                Height: rect[3] as f32,
                MinDepth: 0.0,
                MaxDepth: 1.0,
            }]));
        }
        self.set_scissor(rect);
    }

    pub fn set_scissor(&self, rect: [u32; 4]) {
        let rect = RECT {
            left: rect[0] as i32,
            top: rect[1] as i32,
            right: (rect[0] + rect[2]) as i32,
            bottom: (rect[1] + rect[3]) as i32,
        };
        // SAFETY: the rectangle is copied by the call.
        unsafe { self.context.RSSetScissorRects(Some(&[rect])) };
    }

    pub fn clear(&self, target: &RenderTarget, color: [f32; 4]) {
        // SAFETY: the view lives through the call.
        unsafe { self.context.ClearRenderTargetView(target, &color) };
    }

    /// The state every draw of the renderer shares: triangle strips of four vertices, one instance per record.
    pub fn begin_draws(&self, state: &DrawStates) {
        // SAFETY: the state objects live through the calls.
        unsafe {
            self.context.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP);
            self.context.RSSetState(&state.raster);
        }
    }

    /// Draws `count` instances of a quad, the instance records coming from `buffer`.
    pub fn draw_instances(&self, buffer: &Buffer, stride: u32, count: u32) {
        self.draw_instances_from(buffer, stride, 0..count);
    }

    /// Draws the instances `range` of `buffer`.
    pub fn draw_instances_from(&self, buffer: &Buffer, stride: u32, range: std::ops::Range<u32>) {
        let offset = 0u32;
        // SAFETY: the pointers refer to locals that live through the calls.
        unsafe {
            self.context.IASetVertexBuffers(0, 1, Some(&Some(buffer.clone())), Some(&stride), Some(&offset));
            self.context.DrawInstanced(4, range.end - range.start, 0, range.start);
        }
    }
}

/// Rasterizer and blend state shared by the renderer and the UI layer.
pub struct DrawStates {
    pub raster: ID3D11RasterizerState,
    /// Source over, premultiplied alpha.
    pub premultiplied: ID3D11BlendState,
    /// Per-channel coverage in the second pixel shader output (LCD text).
    pub dual_source: ID3D11BlendState,
}

impl DrawStates {
    pub fn new(gpu: &Gpu) -> Result<DrawStates, String> {
        let raster_desc = D3D11_RASTERIZER_DESC {
            FillMode: D3D11_FILL_SOLID,
            CullMode: D3D11_CULL_NONE,
            ScissorEnable: true.into(),
            DepthClipEnable: true.into(),
            ..Default::default()
        };
        let blend = |color: (D3D11_BLEND, D3D11_BLEND), alpha: (D3D11_BLEND, D3D11_BLEND)| {
            let mut desc = D3D11_BLEND_DESC::default();
            desc.RenderTarget[0] = D3D11_RENDER_TARGET_BLEND_DESC {
                BlendEnable: true.into(),
                SrcBlend: color.0,
                DestBlend: color.1,
                BlendOp: D3D11_BLEND_OP_ADD,
                SrcBlendAlpha: alpha.0,
                DestBlendAlpha: alpha.1,
                BlendOpAlpha: D3D11_BLEND_OP_ADD,
                RenderTargetWriteMask: D3D11_COLOR_WRITE_ENABLE_ALL.0 as u8,
            };
            desc
        };
        let premultiplied = blend((D3D11_BLEND_ONE, D3D11_BLEND_INV_SRC_ALPHA), (D3D11_BLEND_ONE, D3D11_BLEND_INV_SRC_ALPHA));
        let dual_source = blend((D3D11_BLEND_SRC1_COLOR, D3D11_BLEND_INV_SRC1_COLOR), (D3D11_BLEND_ZERO, D3D11_BLEND_ONE));
        // SAFETY: the descriptors live through the calls.
        unsafe {
            let mut raster = None;
            gpu.device.CreateRasterizerState(&raster_desc, Some(&mut raster)).map_err(|e| format!("CreateRasterizerState: {e}"))?;
            let mut premul = None;
            gpu.device.CreateBlendState(&premultiplied, Some(&mut premul)).map_err(|e| format!("CreateBlendState: {e}"))?;
            let mut dual = None;
            gpu.device.CreateBlendState(&dual_source, Some(&mut dual)).map_err(|e| format!("CreateBlendState: {e}"))?;
            Ok(DrawStates {
                raster: raster.ok_or("no rasterizer state")?,
                premultiplied: premul.ok_or("no blend state")?,
                dual_source: dual.ok_or("no blend state")?,
            })
        }
    }

    pub fn set_blend(&self, gpu: &Gpu, dual_source: bool) {
        let blend = if dual_source { &self.dual_source } else { &self.premultiplied };
        // SAFETY: the state lives through the call.
        unsafe { gpu.context.OMSetBlendState(blend, Some(&[1.0; 4]), 0xFFFF_FFFF) };
    }
}

/// HLSL source to bytecode (`entry` compiled for `target`, e.g. `"ps_5_0"`).
pub fn compile(source: &str, entry: &'static str, target: &'static str) -> Result<Vec<u8>, String> {
    let entry_c = format!("{entry}\0");
    let target_c = format!("{target}\0");
    let mut blob = None;
    let mut errors = None;
    // SAFETY: the source and the NUL-terminated names live through the call.
    let result = unsafe {
        D3DCompile(
            source.as_ptr().cast(),
            source.len(),
            None,
            None,
            None,
            PCSTR(entry_c.as_ptr()),
            PCSTR(target_c.as_ptr()),
            0,
            0,
            &mut blob,
            Some(&mut errors),
        )
    };
    if let Err(e) = result {
        let detail = errors
            .map(|b| {
                // SAFETY: the blob owns its buffer.
                let bytes = unsafe { std::slice::from_raw_parts(b.GetBufferPointer().cast::<u8>(), b.GetBufferSize()) };
                String::from_utf8_lossy(bytes).into_owned()
            })
            .unwrap_or_default();
        return Err(format!("{entry} ({target}): {e} {detail}"));
    }
    let blob = blob.ok_or("no bytecode")?;
    // SAFETY: the blob owns its buffer.
    Ok(unsafe { std::slice::from_raw_parts(blob.GetBufferPointer().cast::<u8>(), blob.GetBufferSize()) }.to_vec())
}

/// The bytecode of a vertex shader and of its pixel shader, compiled when building (`build.rs`).
#[derive(Clone, Copy)]
pub struct Shader {
    pub vs: &'static [u8],
    pub ps: &'static [u8],
}

/// A vertex shader, a pixel shader and the layout of the instance records that feed them. The blend state is set apart
/// (`DrawStates::set_blend`).
pub struct Program {
    layout: ID3D11InputLayout,
    vs: ID3D11VertexShader,
    ps: ID3D11PixelShader,
}

fn dxgi_format(format: Format) -> DXGI_FORMAT {
    match format {
        Format::Float32x4 => DXGI_FORMAT_R32G32B32A32_FLOAT,
        Format::Float32x2 => DXGI_FORMAT_R32G32_FLOAT,
        Format::Sint32x2 => DXGI_FORMAT_R32G32_SINT,
        Format::Uint16x2 => DXGI_FORMAT_R16G16_UINT,
        Format::Unorm8x4 => DXGI_FORMAT_R8G8B8A8_UNORM,
        Format::Uint32 => DXGI_FORMAT_R32_UINT,
    }
}

impl Program {
    /// `attributes` are the semantics `ATTR0`, `ATTR1`, ... of the vertex shader, in order, one instance per record of
    /// `stride` bytes.
    pub fn new(gpu: &Gpu, shader: &Shader, attributes: &[Attribute], _stride: u32) -> Result<Program, String> {
        let mut vs = None;
        let mut ps = None;
        // SAFETY: the bytecode lives through the calls.
        unsafe {
            gpu.device.CreateVertexShader(shader.vs, None, Some(&mut vs)).map_err(|e| format!("CreateVertexShader: {e}"))?;
            gpu.device.CreatePixelShader(shader.ps, None, Some(&mut ps)).map_err(|e| format!("CreatePixelShader: {e}"))?;
        }
        let elements: Vec<D3D11_INPUT_ELEMENT_DESC> = attributes
            .iter()
            .enumerate()
            .map(|(i, Attribute(format, offset))| D3D11_INPUT_ELEMENT_DESC {
                SemanticName: PCSTR(c"ATTR".as_ptr().cast()),
                SemanticIndex: i as u32,
                Format: dxgi_format(*format),
                InputSlot: 0,
                AlignedByteOffset: *offset,
                InputSlotClass: D3D11_INPUT_PER_INSTANCE_DATA,
                InstanceDataStepRate: 1,
            })
            .collect();
        let mut layout = None;
        // SAFETY: the descriptors and the bytecode live through the call.
        unsafe { gpu.device.CreateInputLayout(&elements, shader.vs, Some(&mut layout)) }.map_err(|e| format!("CreateInputLayout: {e}"))?;
        Ok(Program {
            layout: layout.ok_or("no input layout")?,
            vs: vs.ok_or("no vertex shader")?,
            ps: ps.ok_or("no pixel shader")?,
        })
    }
}

pub fn set_program(gpu: &Gpu, program: &Program) {
    // SAFETY: the objects live through the calls.
    unsafe {
        gpu.context.IASetInputLayout(&program.layout);
        gpu.context.VSSetShader(&program.vs, None);
        gpu.context.PSSetShader(&program.ps, None);
    }
}

/// A buffer the CPU rewrites wholesale: `Map` with discard. An error when the device cannot make it (it was removed).
pub fn dynamic_buffer(gpu: &Gpu, kind: BufferKind, size: u64) -> Result<Buffer, String> {
    let bind = match kind {
        BufferKind::Vertex => D3D11_BIND_VERTEX_BUFFER,
        BufferKind::Constant => D3D11_BIND_CONSTANT_BUFFER,
    };
    let desc = D3D11_BUFFER_DESC {
        ByteWidth: size as u32,
        Usage: D3D11_USAGE_DYNAMIC,
        BindFlags: bind.0 as u32,
        CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
        ..Default::default()
    };
    let mut buffer = None;
    // SAFETY: the descriptor lives through the call.
    unsafe { gpu.device.CreateBuffer(&desc, None, Some(&mut buffer)) }.map_err(|e| format!("CreateBuffer: {e}"))?;
    buffer.ok_or_else(|| "CreateBuffer returned no buffer".into())
}

pub fn constant_buffer(gpu: &Gpu, size: usize) -> Result<Buffer, String> {
    dynamic_buffer(gpu, BufferKind::Constant, size.next_multiple_of(16) as u64)
}

/// Replaces the contents of a dynamic buffer (`bytes` must not be longer than the buffer).
pub fn write_buffer(gpu: &Gpu, buffer: &Buffer, bytes: &[u8]) {
    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
    // SAFETY: the mapped pointer is valid for the buffer size until `Unmap`, and `bytes` fits in it.
    unsafe {
        if gpu.context.Map(buffer, 0, D3D11_MAP_WRITE_DISCARD, 0, Some(&mut mapped)).is_err() {
            return;
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), mapped.pData.cast::<u8>(), bytes.len());
        gpu.context.Unmap(buffer, 0);
    }
}

pub fn bind_uniforms(gpu: &Gpu, buffer: &Buffer) {
    // SAFETY: the buffer lives through the calls.
    unsafe {
        gpu.context.VSSetConstantBuffers(0, Some(&[Some(buffer.clone())]));
        gpu.context.PSSetConstantBuffers(0, Some(&[Some(buffer.clone())]));
    }
}

/// A 2D texture of `size` x `size` texels that is only sampled, with a view of it.
pub fn atlas_texture(gpu: &Gpu, format: TexelFormat, size: u32) -> Result<(Texture, TextureView), String> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: size,
        Height: size,
        MipLevels: 1,
        ArraySize: 1,
        Format: match format {
            TexelFormat::R8 => DXGI_FORMAT_R8_UNORM,
            TexelFormat::Rgba8 => DXGI_FORMAT_R8G8B8A8_UNORM,
        },
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
        ..Default::default()
    };
    let mut texture = None;
    let mut view = None;
    // SAFETY: the descriptor lives through the calls.
    unsafe {
        gpu.device.CreateTexture2D(&desc, None, Some(&mut texture)).map_err(|e| format!("CreateTexture2D: {e}"))?;
        let texture = texture.ok_or("CreateTexture2D returned no texture")?;
        gpu.device.CreateShaderResourceView(&texture, None, Some(&mut view)).map_err(|e| format!("CreateShaderResourceView: {e}"))?;
        Ok((texture, view.ok_or("CreateShaderResourceView returned no view")?))
    }
}

/// A picture's texture: `width` x `height` RGBA8 texels, `pixels` being their rows without padding.
pub fn picture_texture(gpu: &Gpu, width: u32, height: u32, pixels: &[u8]) -> Option<TextureView> {
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
    let data = D3D11_SUBRESOURCE_DATA { pSysMem: pixels.as_ptr().cast(), SysMemPitch: width * 4, SysMemSlicePitch: 0 };
    let mut texture = None;
    let mut view = None;
    // SAFETY: the descriptor and the pixels live through the calls; the view keeps the texture alive.
    unsafe {
        gpu.device.CreateTexture2D(&desc, Some(&data), Some(&mut texture)).ok()?;
        gpu.device.CreateShaderResourceView(&texture?, None, Some(&mut view)).ok()?;
    }
    view
}

/// Binds a picture's texture, to be drawn with the bilinear sampler.
pub fn bind_picture(gpu: &Gpu, view: &TextureView) {
    // SAFETY: the sampler and the view live through the calls.
    unsafe {
        gpu.context.PSSetSamplers(0, Some(&[Some(gpu.picture_sampler.clone())]));
        gpu.context.PSSetShaderResources(2, Some(&[Some(view.clone())]));
    }
}

/// Writes `pixels` (rows of `pitch` bytes, starting at the box's corner) into the box `rect` (x0, y0, x1, y1, exclusive
/// max) of `texture`.
pub fn upload_texels(gpu: &Gpu, texture: &Texture, pixels: &[u8], pitch: u32, rect: (u32, u32, u32, u32)) {
    let (x0, y0, x1, y1) = rect;
    assert!(pixels.len() >= (pitch * (y1 - y0)) as usize, "the pixels cover the box");
    let region = D3D11_BOX { left: x0, top: y0, front: 0, right: x1, bottom: y1, back: 1 };
    // SAFETY: `pixels` covers every row of the box at `pitch` (checked above) and lives through the call.
    unsafe { gpu.context.UpdateSubresource(texture, 0, Some(&region), pixels.as_ptr().cast::<c_void>(), pitch, 0) };
}

/// Copies the whole of `from` into the top-left corner of the larger `to`.
pub fn copy_texture(gpu: &Gpu, to: &Texture, from: &Texture) {
    // SAFETY: both textures live through the call.
    unsafe { gpu.context.CopySubresourceRegion(to, 0, 0, 0, 0, from, 0, None) };
}

pub fn bind_textures(gpu: &Gpu, views: &[&TextureView]) {
    let views: Vec<Option<ID3D11ShaderResourceView>> = views.iter().map(|v| Some((*v).clone())).collect();
    // SAFETY: the views live through the call.
    unsafe { gpu.context.PSSetShaderResources(0, Some(&views)) };
}

#[cfg(test)]
mod shader_tests {
    /// The shaders are compiled when the window starts (about 20 ms), so a mistake in one would show only then: this makes it
    /// show in the tests.
    #[test]
    fn every_entry_of_the_grid_shaders_compiles() {
        let glyph = include_str!("shaders/glyph.hlsl");
        let rect = include_str!("shaders/rect.hlsl");
        for (source, entry, target) in [
            (rect, "vs_main", "vs_5_0"),
            (glyph, "vs_main", "vs_5_0"),
            (rect, "fs_main", "ps_5_0"),
            (glyph, "fs_main", "ps_5_0"),
            (glyph, "fs_lcd", "ps_5_0"),
        ] {
            let code = super::compile(source, entry, target).unwrap_or_else(|e| panic!("{e}"));
            assert!(code.starts_with(b"DXBC"), "{entry} did not give D3D bytecode");
        }
    }
}
#[cfg(test)]
mod adapter_tests {
    use super::*;
    use windows::Win32::{
        Foundation::POINT,
        Graphics::Gdi::{MONITOR_DEFAULTTOPRIMARY, MonitorFromPoint},
    };

    #[test]
    fn the_adapter_of_the_primary_monitor_is_found_and_an_unknown_monitor_falls_back_to_the_low_power_one() {
        // SAFETY: a plain query.
        let primary = unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) };
        let driving = Gpu::new(AdapterKind::Display(primary.0 as isize)).expect("the adapter of the primary monitor");
        let fallback = Gpu::new(AdapterKind::Display(0)).expect("the low-power adapter");
        let low_power = Gpu::new(AdapterKind::Real).expect("the low-power adapter");
        assert!(!driving.adapter_name.is_empty());
        assert_eq!(fallback.adapter_name, low_power.adapter_name, "no monitor is no reason to pick another adapter");
    }
}
