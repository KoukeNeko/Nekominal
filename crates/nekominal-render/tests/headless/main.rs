//! Headless (WARP) acceptance tests for the renderer: colour resolution, translucent compositing, cursor and
//! focus rendering, goldens, and atlas memory. One binary so there is one DX12 device per run.

mod atlas;
mod colors;
mod common;
mod cursor;
mod golden;
mod images;
mod ligatures;
mod translucent;
