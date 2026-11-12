//! The floating panels of a window are drawn in the window's own frame here: no compositor lets a program put a blurred
//! layer of its own over its window, so the layer of the Windows build is never made and `open` says so (the application
//! then draws the panels itself, as it does where a layer cannot be made).

use nekominal_ui::Rect;

use crate::{
    gpu::{Gpu, Surface},
    win::Hwnd,
};

pub struct Layer {
    pub hwnd: Hwnd,
    pub surface: Surface,
}

pub fn open(_owner: Hwnd, _gpu: &Gpu, _dark: bool) -> Result<Layer, String> {
    Err("panels are drawn in the window".into())
}

impl Layer {
    pub fn place(&mut self, _owner: Hwnd, _gpu: &Gpu, _size: (u32, u32), _panels: &[(Rect, f32)], _scale: f32) {}

    pub fn follow(&self, _owner: Hwnd) {}

    pub fn show(&mut self) {}

    pub fn hide(&mut self) -> bool {
        false
    }

    pub fn shrink(&mut self, _gpu: &Gpu) {}

    pub fn shown(&self) -> bool {
        false
    }
}

pub fn cursor_on_layer(_layer: Hwnd) -> bool {
    false
}
