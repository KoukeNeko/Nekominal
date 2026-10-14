//! Nekominal terminal grid renderer spike (wgpu 30, DX12 first).
//!
//! Layering:
//! * `cell`      input data model (cells, theme, overlays) - no GPU types
//! * `contrast`  WCAG contrast math for `Theme::min_contrast`
//! * `fonts`     font stack, fallback, cell metrics (DirectWrite discovery)
//! * `raster`    swash rasterization (grayscale / LCD / color / synthetic styles)
//! * `sprites`   procedural box drawing, blocks, braille, Powerline, underlines
//! * `atlas`     etagere-packed R8 + RGBA8 glyph atlases
//! * `gpu`       instance structs, uniforms, pipelines
//! * `renderer`  public `Renderer` / `PaneState` API; `rows` implements the update path
//! * `offscreen` headless target, adapter selection and PNG output for tests (cargo feature `offscreen`)

pub mod atlas;
pub mod cell;
pub mod contrast;
pub mod fonts;
pub mod gpu;
#[cfg(windows)]
pub mod locator;
#[cfg(feature = "offscreen")]
pub mod offscreen;
pub mod raster;
pub mod renderer;
mod rows;
pub mod sprites;

pub use cell::*;
pub use renderer::{PaneDraw, PaneState, Renderer, RendererConfig, UpdateStats};
