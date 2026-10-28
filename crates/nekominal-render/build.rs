//! Compiles the HLSL shaders of the renderer and of the interface (`nekominal-ui`) to bytecode when building, so the
//! program does not load the shader compiler (`d3dcompiler_47.dll`) and compile them at every start. `src/shaders.rs`
//! includes what this writes: a constant of D3D bytecode for each entry.

use std::{fmt::Write as _, path::Path};

use windows::{
    Win32::Graphics::Direct3D::Fxc::D3DCompile,
    core::PCSTR,
};

/// The shaders: the constant's name, the file (from this crate's folder), the entry point and the target.
const SHADERS: &[(&str, &str, &str, &str)] = &[
    ("RECT_VS", "src/shaders/rect.hlsl", "vs_main", "vs_5_0"),
    ("RECT_PS", "src/shaders/rect.hlsl", "fs_main", "ps_5_0"),
    ("GLYPH_VS", "src/shaders/glyph.hlsl", "vs_main", "vs_5_0"),
    ("GLYPH_PS", "src/shaders/glyph.hlsl", "fs_main", "ps_5_0"),
    ("GLYPH_LCD_PS", "src/shaders/glyph.hlsl", "fs_lcd", "ps_5_0"),
    ("IMAGE_VS", "src/shaders/image.hlsl", "vs_main", "vs_5_0"),
    ("IMAGE_PS", "src/shaders/image.hlsl", "fs_main", "ps_5_0"),
    ("UI_QUAD_VS", "../nekominal-ui/src/quad.hlsl", "vs_main", "vs_5_0"),
    ("UI_QUAD_PS", "../nekominal-ui/src/quad.hlsl", "fs_main", "ps_5_0"),
    ("UI_GLYPH_VS", "../nekominal-ui/src/glyph.hlsl", "vs_main", "vs_5_0"),
    ("UI_GLYPH_PS", "../nekominal-ui/src/glyph.hlsl", "fs_main", "ps_5_0"),
];

fn compile(source: &str, entry: &str, target: &str) -> Result<Vec<u8>, String> {
    let (entry, target) = (format!("{entry}\0"), format!("{target}\0"));
    let (mut blob, mut errors) = (None, None);
    // SAFETY: the source and the NUL-terminated names live through the call; the blobs own their buffers.
    unsafe {
        let result = D3DCompile(source.as_ptr().cast(), source.len(), None, None, None, PCSTR(entry.as_ptr()), PCSTR(target.as_ptr()), 0, 0, &mut blob, Some(&mut errors));
        let bytes = |b: &windows::Win32::Graphics::Direct3D::ID3DBlob| std::slice::from_raw_parts(b.GetBufferPointer().cast::<u8>(), b.GetBufferSize()).to_vec();
        match (result, blob) {
            (Ok(()), Some(blob)) => Ok(bytes(&blob)),
            (result, _) => Err(format!("{result:?}: {}", errors.map(|e| String::from_utf8_lossy(&bytes(&e)).into_owned()).unwrap_or_default())),
        }
    }
}

fn main() {
    let out = std::env::var("OUT_DIR").unwrap();
    let mut index = String::new();
    for (name, file, entry, target) in SHADERS {
        println!("cargo:rerun-if-changed={file}");
        let source = std::fs::read_to_string(file).unwrap_or_else(|e| panic!("{file}: {e}"));
        let code = compile(&source, entry, target).unwrap_or_else(|e| panic!("{file} {entry}: {e}"));
        let path = Path::new(&out).join(format!("{name}.cso"));
        std::fs::write(&path, code).unwrap();
        writeln!(index, "pub const {name}: &[u8] = include_bytes!(r\"{}\");", path.display()).unwrap();
    }
    std::fs::write(Path::new(&out).join("shaders.rs"), index).unwrap();
}
