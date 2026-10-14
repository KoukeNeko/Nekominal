//! HWND and screen-geometry helpers for a WinUI 3 window.

use nekominal_winui_sys::*;
use windows_core::{Interface, Result};

pub fn hwnd(window: &Window) -> Result<windows::Win32::Foundation::HWND> {
    let native: IWindowNative = window.cast()?;
    let mut raw = std::ptr::null_mut();
    unsafe { native.WindowHandle(&mut raw).ok()? };
    Ok(windows::Win32::Foundation::HWND(raw))
}

/// Screen rectangle (x, y, width, height in physical pixels) of `el`: client origin of the window plus the
/// element's transform to `root` times the rasterization scale.
pub fn element_screen_rect(window: &Window, root: &FrameworkElement, el: &FrameworkElement) -> Option<(f32, f32, f32, f32)> {
    use windows::Win32::Foundation::POINT;
    let scale = root.cast::<UIElement>().ok()?.XamlRoot().ok()?.RasterizationScale().ok()? as f32;
    let p = el.cast::<UIElement>().ok()?.TransformToVisual(root).ok()?.TransformPoint(Point { x: 0.0, y: 0.0 }).ok()?;
    let hwnd = hwnd(window).ok()?;
    let mut origin = POINT { x: 0, y: 0 };
    unsafe {
        let _ = windows::Win32::Graphics::Gdi::ClientToScreen(hwnd, &mut origin);
    }
    Some((
        origin.x as f32 + p.x * scale,
        origin.y as f32 + p.y * scale,
        el.ActualWidth().ok()? as f32 * scale,
        el.ActualHeight().ok()? as f32 * scale,
    ))
}
