//! The Direct3D 11 side of the renderer: the device and its immediate context, and the small helpers (buffers,
//! textures, shaders, draw state) the renderer and the UI layer are built from. One `Gpu` is used from one thread.

use std::ffi::c_void;

use windows::{
    Win32::{
        Foundation::{HMODULE, RECT},
        Graphics::{
            Direct3D::{
                D3D_DRIVER_TYPE_UNKNOWN, D3D_DRIVER_TYPE_WARP, D3D_FEATURE_LEVEL_11_0, D3D_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP,
                Fxc::D3DCompile,
            },
            Direct3D11::*,
            Dxgi::{Common::DXGI_FORMAT, *},
        },
    },
    core::{Interface, PCSTR},
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AdapterKind {
    /// The low-power hardware adapter (the integrated GPU where there are two).
    Real,
    /// Microsoft Basic Render Driver (software rasterizer).
    Warp,
}

#[derive(Clone)]
pub struct Gpu {
    pub device: ID3D11Device,
    pub context: ID3D11DeviceContext,
    pub adapter_name: String,
}

impl Gpu {
    pub fn new(kind: AdapterKind) -> Result<Gpu, String> {
        // SAFETY: plain DXGI/D3D11 creation calls; every pointer outlives its call.
        unsafe {
            let mut adapter_name = String::from("WARP");
            let (adapter, driver) = match kind {
                AdapterKind::Warp => (None, D3D_DRIVER_TYPE_WARP),
                AdapterKind::Real => {
                    let factory: IDXGIFactory6 = CreateDXGIFactory1().map_err(|e| format!("CreateDXGIFactory1: {e}"))?;
                    let adapter: IDXGIAdapter1 = factory
                        .EnumAdapterByGpuPreference(0, DXGI_GPU_PREFERENCE_MINIMUM_POWER)
                        .map_err(|e| format!("no hardware adapter: {e}"))?;
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
            Ok(Gpu { device: device.ok_or("no device")?, context: context.ok_or("no context")?, adapter_name })
        }
    }

    /// Makes `target` the render target, with the viewport and the scissor rectangle both set to `rect`
    /// (x, y, width, height in pixels).
    pub fn bind_target(&self, target: &ID3D11RenderTargetView, rect: [u32; 4]) {
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

    pub fn clear(&self, target: &ID3D11RenderTargetView, color: [f32; 4]) {
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
    pub fn draw_instances(&self, buffer: &ID3D11Buffer, stride: u32, count: u32) {
        self.draw_instances_from(buffer, stride, 0..count);
    }

    /// Draws the instances `range` of `buffer`.
    pub fn draw_instances_from(&self, buffer: &ID3D11Buffer, stride: u32, range: std::ops::Range<u32>) {
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

pub fn vertex_shader(gpu: &Gpu, code: &[u8]) -> Result<ID3D11VertexShader, String> {
    let mut shader = None;
    // SAFETY: the bytecode lives through the call.
    unsafe { gpu.device.CreateVertexShader(code, None, Some(&mut shader)) }.map_err(|e| format!("CreateVertexShader: {e}"))?;
    shader.ok_or_else(|| "no vertex shader".into())
}

pub fn pixel_shader(gpu: &Gpu, code: &[u8]) -> Result<ID3D11PixelShader, String> {
    let mut shader = None;
    // SAFETY: the bytecode lives through the call.
    unsafe { gpu.device.CreatePixelShader(code, None, Some(&mut shader)) }.map_err(|e| format!("CreatePixelShader: {e}"))?;
    shader.ok_or_else(|| "no pixel shader".into())
}

/// One attribute of an instance record: its format and byte offset. The attributes are the semantics `ATTR0`,
/// `ATTR1`, ... of the vertex shader, in order.
pub struct Attribute(pub DXGI_FORMAT, pub u32);

pub fn input_layout(gpu: &Gpu, vertex_code: &[u8], attributes: &[Attribute]) -> Result<ID3D11InputLayout, String> {
    let elements: Vec<D3D11_INPUT_ELEMENT_DESC> = attributes
        .iter()
        .enumerate()
        .map(|(i, Attribute(format, offset))| D3D11_INPUT_ELEMENT_DESC {
            SemanticName: PCSTR(c"ATTR".as_ptr().cast()),
            SemanticIndex: i as u32,
            Format: *format,
            InputSlot: 0,
            AlignedByteOffset: *offset,
            InputSlotClass: D3D11_INPUT_PER_INSTANCE_DATA,
            InstanceDataStepRate: 1,
        })
        .collect();
    let mut layout = None;
    // SAFETY: the descriptors and the bytecode live through the call.
    unsafe { gpu.device.CreateInputLayout(&elements, vertex_code, Some(&mut layout)) }.map_err(|e| format!("CreateInputLayout: {e}"))?;
    layout.ok_or_else(|| "no input layout".into())
}

/// A buffer the CPU rewrites wholesale: `Map` with discard.
pub fn dynamic_buffer(gpu: &Gpu, bind: D3D11_BIND_FLAG, size: u64) -> ID3D11Buffer {
    let desc = D3D11_BUFFER_DESC {
        ByteWidth: size as u32,
        Usage: D3D11_USAGE_DYNAMIC,
        BindFlags: bind.0 as u32,
        CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
        ..Default::default()
    };
    let mut buffer = None;
    // SAFETY: the descriptor lives through the call.
    unsafe { gpu.device.CreateBuffer(&desc, None, Some(&mut buffer)) }.expect("CreateBuffer");
    buffer.expect("CreateBuffer returned no buffer")
}

pub fn constant_buffer(gpu: &Gpu, size: usize) -> ID3D11Buffer {
    dynamic_buffer(gpu, D3D11_BIND_CONSTANT_BUFFER, size.next_multiple_of(16) as u64)
}

/// Replaces the contents of a dynamic buffer (`bytes` must not be longer than the buffer).
pub fn write_buffer(gpu: &Gpu, buffer: &ID3D11Buffer, bytes: &[u8]) {
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

pub fn bind_uniforms(gpu: &Gpu, buffer: &ID3D11Buffer) {
    // SAFETY: the buffer lives through the calls.
    unsafe {
        gpu.context.VSSetConstantBuffers(0, Some(&[Some(buffer.clone())]));
        gpu.context.PSSetConstantBuffers(0, Some(&[Some(buffer.clone())]));
    }
}

/// A 2D texture of `size` x `size` texels that is only sampled, with a view of it.
pub fn atlas_texture(gpu: &Gpu, format: DXGI_FORMAT, size: u32) -> (ID3D11Texture2D, ID3D11ShaderResourceView) {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: size,
        Height: size,
        MipLevels: 1,
        ArraySize: 1,
        Format: format,
        SampleDesc: windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
        ..Default::default()
    };
    let mut texture = None;
    let mut view = None;
    // SAFETY: the descriptor lives through the calls.
    unsafe {
        gpu.device.CreateTexture2D(&desc, None, Some(&mut texture)).expect("CreateTexture2D");
        let texture = texture.expect("CreateTexture2D returned no texture");
        gpu.device.CreateShaderResourceView(&texture, None, Some(&mut view)).expect("CreateShaderResourceView");
        (texture, view.expect("CreateShaderResourceView returned no view"))
    }
}

/// Copies the texels of `rect` (x0, y0, x1, y1, exclusive max) from `shadow`, whose rows are `pitch` bytes apart.
pub fn upload_texels(gpu: &Gpu, texture: &ID3D11Texture2D, shadow: &[u8], pitch: u32, bytes_per_texel: u32, rect: (u32, u32, u32, u32)) {
    let (x0, y0, x1, y1) = rect;
    let region = D3D11_BOX { left: x0, top: y0, front: 0, right: x1, bottom: y1, back: 1 };
    let offset = (y0 * pitch + x0 * bytes_per_texel) as usize;
    // SAFETY: the source pointer is inside `shadow`, which covers every row of the box at `pitch`.
    unsafe { gpu.context.UpdateSubresource(texture, 0, Some(&region), shadow[offset..].as_ptr().cast::<c_void>(), pitch, 0) };
}

/// Copies the whole of `from` into the top-left corner of the larger `to`.
pub fn copy_texture(gpu: &Gpu, to: &ID3D11Texture2D, from: &ID3D11Texture2D) {
    // SAFETY: both textures live through the call.
    unsafe { gpu.context.CopySubresourceRegion(to, 0, 0, 0, 0, from, 0, None) };
}

pub fn bind_textures(gpu: &Gpu, views: &[&ID3D11ShaderResourceView]) {
    let views: Vec<Option<ID3D11ShaderResourceView>> = views.iter().map(|v| Some((*v).clone())).collect();
    // SAFETY: the views live through the call.
    unsafe { gpu.context.PSSetShaderResources(0, Some(&views)) };
}

pub fn set_shaders(gpu: &Gpu, layout: &ID3D11InputLayout, vs: &ID3D11VertexShader, ps: &ID3D11PixelShader) {
    // SAFETY: the objects live through the calls.
    unsafe {
        gpu.context.IASetInputLayout(layout);
        gpu.context.VSSetShader(vs, None);
        gpu.context.PSSetShader(ps, None);
    }
}
