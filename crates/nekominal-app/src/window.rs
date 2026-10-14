//! The top-level window: system backdrop, title bar and the window theme.

use nekominal_model::settings::Backdrop;
use nekominal_winui_sys::*;
use windows_core::{Interface, Result};

use crate::glass::Glass;
use crate::util::{app_window, argb, log, solid_brush, xaml_color};

/// Sets what shows behind (and through) the glass: a real blur of the desktop, Mica, Mica Alt, or nothing.
/// Without a backdrop the root gets an opaque scheme-coloured background instead.
pub fn apply_backdrop(window: &Window, root: &FrameworkElement, backdrop: Backdrop, glass: &Glass) -> Result<()> {
    let window2: IWindow2 = window.cast()?;
    match backdrop {
        Backdrop::Acrylic => window2.SetSystemBackdrop(&DesktopAcrylicBackdrop::new()?.cast::<SystemBackdrop>()?)?,
        Backdrop::Mica | Backdrop::MicaAlt => {
            let mica = MicaBackdrop::new()?;
            mica.SetKind(if backdrop == Backdrop::Mica { MicaKind::Base } else { MicaKind::BaseAlt })?;
            window2.SetSystemBackdrop(&mica.cast::<SystemBackdrop>()?)?;
        }
        Backdrop::None => window2.SetSystemBackdrop(None::<&SystemBackdrop>)?,
    }
    let panel: Panel = root.cast()?;
    if backdrop == Backdrop::None {
        panel.SetBackground(&solid_brush(glass.background)?)?;
    } else {
        panel.SetBackground(None::<&Brush>)?;
    }
    Ok(())
}

/// Window theme (flyouts, menus, dialogs) and caption button colours follow the colour scheme.
pub fn apply_theme(window: &Window, root: &FrameworkElement, glass: &Glass) -> Result<()> {
    root.SetRequestedTheme(if glass.light { ElementTheme::Light } else { ElementTheme::Dark })?;
    let title_bar = app_window(window)?.TitleBar()?;
    title_bar.cast::<IAppWindowTitleBar3>()?.SetPreferredTheme(if glass.light {
        TitleBarTheme::Light
    } else {
        TitleBarTheme::Dark
    })?;
    let text = xaml_color(glass.text());
    let hover = xaml_color(glass.hover());
    let pressed = xaml_color(glass.pressed());
    let clear = argb(0, 0, 0, 0);
    title_bar.SetButtonBackgroundColor(Some(clear))?;
    title_bar.SetButtonInactiveBackgroundColor(Some(clear))?;
    title_bar.SetButtonForegroundColor(Some(text))?;
    title_bar.SetButtonHoverForegroundColor(Some(text))?;
    title_bar.SetButtonPressedForegroundColor(Some(text))?;
    title_bar.SetButtonHoverBackgroundColor(Some(hover))?;
    title_bar.SetButtonPressedBackgroundColor(Some(pressed))?;
    title_bar.SetButtonInactiveForegroundColor(Some(xaml_color(glass.text_secondary())))?;
    Ok(())
}

/// Extends the content into the title bar (48 DIP, the native caption buttons stay and snap layouts keep
/// working) and makes `drag` the only caption area.
pub fn extend_into_title_bar(window: &Window, drag: &UIElement) -> Result<()> {
    window.SetExtendsContentIntoTitleBar(true)?;
    window.SetTitleBar(drag)?;
    let title_bar = app_window(window)?.TitleBar()?;
    title_bar.cast::<IAppWindowTitleBar2>()?.SetPreferredHeightOption(TitleBarHeightOption::Tall)?;
    Ok(())
}

/// Width of the system caption buttons (right) and the reserved space on the left, in DIP.
pub fn caption_insets(window: &Window, scale: f64) -> (f64, f64) {
    let Ok(title_bar) = app_window(window).and_then(|w| w.TitleBar()) else { return (0.0, 0.0) };
    let left = f64::from(title_bar.LeftInset().unwrap_or(0)) / scale;
    let right = f64::from(title_bar.RightInset().unwrap_or(0)) / scale;
    log(&format!("caption insets left={left:.1} right={right:.1} DIP at scale {scale}"));
    (left, right)
}
