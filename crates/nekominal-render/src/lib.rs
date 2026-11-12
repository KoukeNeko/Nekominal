//! Nekominal terminal grid renderer on Direct3D 11 (Windows) and wgpu (Linux).
//!
//! Layering:
//! * `cell`      input data model (cells, theme, overlays) - no GPU types
//! * `contrast`  WCAG contrast math for `Theme::min_contrast`
//! * `fonts`     font stack, fallback, cell metrics (DirectWrite or fontconfig discovery)
//! * `raster`    swash rasterization (grayscale / LCD / color / synthetic styles)
//! * `ligatures` runs of cells shaped with the font's `calt`/`liga` when ligatures are on
//! * `sprites`   procedural box drawing, blocks, braille, Powerline, underlines
//! * `atlas`     etagere-packed R8 + RGBA8 glyph atlases
//! * `gfx`       the device and the small helpers over it: Direct3D 11 (`d3d.rs`) or wgpu (`wgpu_gfx.rs`)
//! * `gpu`       instance structs, uniforms, pipelines
//! * `images`    pictures over the grid (Sixel, kitty graphics): textures and their quads
//! * `renderer`  public `Renderer` / `PaneState` API; `rows` implements the update path
//! * `offscreen` headless target, adapter selection and PNG output for tests (cargo feature `offscreen`)

pub mod atlas;
pub mod cell;
pub mod contrast;
#[cfg(windows)]
#[path = "d3d.rs"]
pub mod gfx;
#[cfg(unix)]
#[path = "wgpu_gfx.rs"]
pub mod gfx;
pub mod fonts;
pub mod gpu;
mod images;
mod ligatures;
#[cfg_attr(windows, path = "locator.rs")]
#[cfg_attr(unix, path = "locator_unix.rs")]
pub mod locator;
#[cfg(feature = "offscreen")]
#[cfg_attr(windows, path = "offscreen.rs")]
#[cfg_attr(unix, path = "offscreen_wgpu.rs")]
pub mod offscreen;
pub mod raster;
pub mod renderer;
mod rows;
/// The shaders: D3D bytecode compiled by `build.rs` on Windows, WGSL on Linux.
#[cfg(windows)]
pub mod shaders {
    use crate::gfx::Shader;

    include!(concat!(env!("OUT_DIR"), "/shaders.rs"));

    pub const RECT: Shader = Shader { vs: RECT_VS, ps: RECT_PS };
    pub const GLYPH: Shader = Shader { vs: GLYPH_VS, ps: GLYPH_PS };
    pub const GLYPH_LCD: Shader = Shader { vs: GLYPH_VS, ps: GLYPH_LCD_PS };
    pub const IMAGE: Shader = Shader { vs: IMAGE_VS, ps: IMAGE_PS };
    pub const UI_QUAD: Shader = Shader { vs: UI_QUAD_VS, ps: UI_QUAD_PS };
    pub const UI_GLYPH: Shader = Shader { vs: UI_GLYPH_VS, ps: UI_GLYPH_PS };
}
#[cfg(unix)]
pub mod shaders {
    use crate::gfx::Shader;

    const GLYPH_SOURCE: &str = include_str!("shaders/glyph.wgsl");

    pub const RECT: Shader = Shader { sources: &[include_str!("shaders/rect.wgsl")], vs: "vs_main", ps: "fs_main", dual_source: false, erase: false };
    pub const GLYPH: Shader = Shader { sources: &[GLYPH_SOURCE], vs: "vs_main", ps: "fs_main", dual_source: false, erase: false };
    pub const GLYPH_LCD: Shader = Shader { sources: &[GLYPH_SOURCE, include_str!("shaders/glyph_lcd.wgsl")], vs: "vs_main", ps: "fs_lcd", dual_source: true, erase: false };
    pub const IMAGE: Shader = Shader { sources: &[include_str!("shaders/image.wgsl")], vs: "vs_main", ps: "fs_main", dual_source: false, erase: false };
    pub const UI_QUAD: Shader = Shader { sources: &[include_str!("../../nekominal-ui/src/quad.wgsl")], vs: "vs_main", ps: "fs_main", dual_source: false, erase: false };
    /// The quads of the interface that take away what is under them, for the corners of a window with round ones.
    pub const UI_ERASE: Shader = Shader { sources: &[include_str!("../../nekominal-ui/src/quad.wgsl")], vs: "vs_main", ps: "fs_erase", dual_source: false, erase: true };
    pub const UI_GLYPH: Shader = Shader { sources: &[include_str!("../../nekominal-ui/src/glyph.wgsl")], vs: "vs_main", ps: "fs_main", dual_source: false, erase: false };
}
pub mod sprites;

pub use cell::*;
pub use images::ImageView;
pub use renderer::{PaneDraw, PaneState, Renderer, RendererConfig, UpdateStats};
