//! Nekominal terminal grid renderer on Direct3D 11.
//!
//! Layering:
//! * `cell`      input data model (cells, theme, overlays) - no GPU types
//! * `contrast`  WCAG contrast math for `Theme::min_contrast`
//! * `fonts`     font stack, fallback, cell metrics (DirectWrite discovery)
//! * `raster`    swash rasterization (grayscale / LCD / color / synthetic styles)
//! * `ligatures` runs of cells shaped with the font's `calt`/`liga` when ligatures are on
//! * `sprites`   procedural box drawing, blocks, braille, Powerline, underlines
//! * `atlas`     etagere-packed R8 + RGBA8 glyph atlases
//! * `d3d`       the device and the small Direct3D 11 helpers
//! * `gpu`       instance structs, uniforms, pipelines
//! * `images`    pictures over the grid (Sixel, kitty graphics): textures and their quads
//! * `renderer`  public `Renderer` / `PaneState` API; `rows` implements the update path
//! * `offscreen` headless target, adapter selection and PNG output for tests (cargo feature `offscreen`)

pub mod atlas;
pub mod cell;
pub mod contrast;
pub mod d3d;
pub mod fonts;
pub mod gpu;
mod images;
mod ligatures;
#[cfg(windows)]
pub mod locator;
#[cfg(feature = "offscreen")]
pub mod offscreen;
pub mod raster;
pub mod renderer;
mod rows;
/// The D3D bytecode of every shader, compiled by `build.rs`.
pub mod shaders {
    include!(concat!(env!("OUT_DIR"), "/shaders.rs"));
}
pub mod sprites;

pub use cell::*;
pub use images::ImageView;
pub use renderer::{PaneDraw, PaneState, Renderer, RendererConfig, UpdateStats};
