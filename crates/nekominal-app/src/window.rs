//! The top-level window: system backdrop, title bar and the window theme.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use nekominal_model::settings::Backdrop;
use nekominal_winui_sys::*;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    FLASHW_TIMERNOFG, FLASHW_TRAY, FLASHWINFO, FlashWindowEx, GetSystemMetrics, MNC_CLOSE, SM_SHOWSOUNDS,
    SPI_GETCLIENTAREAANIMATION, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW, WM_CLOSE, WM_MENUCHAR,
};
use windows_core::{Interface, Result};

use crate::glass::Glass;
use crate::util::{app_window, argb, log, solid_brush, xaml_color};

/// Identifies the window subclass of [`silence_alt_chords`] so it can be removed again.
const ALT_CHORD_SUBCLASS: usize = 1;

/// How many `WM_MENUCHAR` the subclass has answered. Every one is logged with its number, so a run shows that the
/// subclass sees the messages and how many an Alt chord (or a held key) raised.
static MENU_CHARS: AtomicU32 = AtomicU32::new(0);

/// The user has answered the question a close of the window asked: the `WM_CLOSE` that follows is let through.
static CLOSE_CONFIRMED: AtomicBool = AtomicBool::new(false);

/// Lets the next close of the window through without asking again.
pub fn allow_close() {
    CLOSE_CONFIRMED.store(true, Ordering::Relaxed);
}

/// Alt+character shortcuts (Alt+Shift+D, ...) arrive as `WM_SYSCHAR`. The window has no menu mnemonic for the
/// character, so Windows asks `WM_MENUCHAR`, and the default answer (`MNC_IGNORE`) is a beep from the system
/// speaker. Answering `MNC_CLOSE` keeps the shortcut silent. A `WM_CLOSE` (Alt+F4, the caption button) is held back
/// while the shell asks about the running sessions it would end. Everything else, `WM_SYSCHAR` and `WM_SYSCOMMAND`
/// included, goes on to the window's own procedure, so Alt+Space still opens the system menu.
unsafe extern "system" fn silence_menu_char(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _subclass_id: usize,
    _data: usize,
) -> LRESULT {
    if message == WM_MENUCHAR {
        let count = MENU_CHARS.fetch_add(1, Ordering::Relaxed) + 1;
        log(&format!("WM_MENUCHAR #{count} answered with MNC_CLOSE"));
        // The menu command is the high-order word of the result: MAKELRESULT(0, MNC_CLOSE).
        return LRESULT((MNC_CLOSE as isize) << 16);
    }
    if message == WM_CLOSE
        && !CLOSE_CONFIRMED.load(Ordering::Relaxed)
        && crate::shell::shell().is_some_and(|shell| shell.ask_before_closing_window())
    {
        return LRESULT(0);
    }
    // SAFETY: forwards the message this procedure was called with to the next one in the chain.
    unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
}

/// Subclasses the top-level window so that Alt+character shortcuts do not beep.
pub fn silence_alt_chords(window: &Window) -> Result<()> {
    let hwnd = nekominal_winui::window::hwnd(window)?;
    // SAFETY: `hwnd` is this process's live top-level window, on the thread that created it, and
    // `silence_menu_char` is a plain function that needs no reference data.
    unsafe { SetWindowSubclass(hwnd, Some(silence_menu_char), ALT_CHORD_SUBCLASS, 0) }.ok().map_err(from_win32)
}

/// Takes the subclass of [`silence_alt_chords`] off the window again.
pub fn restore_alt_chords(window: &Window) -> Result<()> {
    let hwnd = nekominal_winui::window::hwnd(window)?;
    // SAFETY: removes only the subclass installed above, from the thread that installed it.
    unsafe { RemoveWindowSubclass(hwnd, Some(silence_menu_char), ALT_CHORD_SUBCLASS) }.ok().map_err(from_win32)
}

/// Flashes the taskbar button until the window comes to the foreground. A window in the foreground does not
/// flash, so this is for the bell of an inactive window.
pub fn flash_taskbar(window: &Window) -> Result<()> {
    let info = FLASHWINFO {
        cbSize: size_of::<FLASHWINFO>() as u32,
        hwnd: nekominal_winui::window::hwnd(window)?,
        dwFlags: FLASHW_TRAY | FLASHW_TIMERNOFG,
        uCount: 0,
        dwTimeout: 0,
    };
    // SAFETY: `info` is a complete FLASHWINFO of this call and names a live window of this process. The result is
    // whether the window was flashing before, not an error.
    let _ = unsafe { FlashWindowEx(&info) };
    Ok(())
}

/// The system asks for sounds to be shown as pictures (`SM_SHOWSOUNDS`).
pub fn show_sounds() -> bool {
    // SAFETY: a plain query of a system metric.
    unsafe { GetSystemMetrics(SM_SHOWSOUNDS) != 0 }
}

/// Animations in the client area are on (the "Animation effects" setting of Windows). A spinner that would move
/// stays still when this is off.
pub fn animations_enabled() -> bool {
    let mut enabled = windows::core::BOOL(1);
    // SAFETY: SPI_GETCLIENTAREAANIMATION writes one BOOL through the pointer, which outlives the call.
    let queried = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            Some(&mut enabled as *mut windows::core::BOOL as *mut _),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    queried.is_err() || enabled.as_bool()
}

/// An error of the `windows` crate as one of `windows_core` (the two crates are different generations).
fn from_win32(error: windows::core::Error) -> windows_core::Error {
    windows_core::Error::new(windows_core::HRESULT(error.code().0), error.message())
}

/// Sets what shows behind (and through) the glass: a real blur of the desktop, Mica, Mica Alt, or nothing.
/// Every call builds a new backdrop and replaces the window's one, so it is made only when the kind changes.
pub fn set_system_backdrop(window: &Window, backdrop: Backdrop) -> Result<()> {
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
    log(&format!("system backdrop set: {backdrop:?}"));
    Ok(())
}

/// Without a backdrop the root gets an opaque scheme-coloured background; with one it stays clear.
pub fn apply_root_background(root: &FrameworkElement, backdrop: Backdrop, glass: &Glass) -> Result<()> {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// What a window procedure answers to `WM_MENUCHAR` for the character `c`, as (menu, command).
    fn menu_char_answer(c: char) -> (u32, u32) {
        // SAFETY: `WM_MENUCHAR` is answered before the next procedure of the subclass chain is reached, so no window
        // behind the handle is touched.
        let result = unsafe {
            silence_menu_char(HWND::default(), WM_MENUCHAR, WPARAM(c as usize), LPARAM(0), ALT_CHORD_SUBCLASS, 0)
        };
        let bits = result.0 as u32;
        (bits & 0xFFFF, bits >> 16)
    }

    #[test]
    fn an_alt_character_is_answered_with_close_in_the_high_word_so_it_does_not_beep() {
        // MAKELRESULT(0, MNC_CLOSE): MNC_CLOSE (1, as documented for WM_MENUCHAR) in the high-order word, menu 0 in
        // the low one. The default answer, MNC_IGNORE (0), is the beep.
        for c in ['d', 's', '=', '-', 'x'] {
            assert_eq!(menu_char_answer(c), (0, 1), "{c}");
        }
        assert_eq!(MNC_CLOSE, 1, "the value the documentation gives MNC_CLOSE");
    }

    #[test]
    fn every_menu_character_is_counted_in_the_log() {
        let before = MENU_CHARS.load(Ordering::Relaxed);
        for c in ['d', 'x', 'q'] {
            menu_char_answer(c);
        }
        assert!(MENU_CHARS.load(Ordering::Relaxed) >= before + 3, "tests share the counter, and only add to it");
    }
}