//! Small helpers shared by the shell: XAML loading, `FindName`, colours, COM identity.

use nekominal_model::themes::Color as ModelColor;
use nekominal_winui_sys::*;
use windows_core::{EventRevoker, Interface, Result};

pub use nekominal_winui::diag::log;

/// `XamlReader::Load` + cast to `FrameworkElement`. The markup root must declare the WinUI default namespace.
pub fn load_xaml(markup: &str) -> Result<FrameworkElement> {
    XamlReader::Load(markup)?.cast()
}

/// `FrameworkElement::FindName` + cast: looks up an `x:Name` in the namescope of a `XamlReader::Load` tree.
pub fn find<T: Interface>(root: &FrameworkElement, name: &str) -> Result<T> {
    root.FindName(name)
        .map_err(|e| windows_core::Error::new(e.code(), format!("x:Name '{name}' not found: {e}")))?
        .cast()
}

pub fn argb(a: u8, r: u8, g: u8, b: u8) -> Color {
    Color { a, r, g, b }
}

pub fn xaml_color(color: ModelColor) -> Color {
    Color { a: color.a, r: color.r, g: color.g, b: color.b }
}

pub fn solid_brush(color: ModelColor) -> Result<SolidColorBrush> {
    let brush = SolidColorBrush::new()?;
    brush.SetColor(xaml_color(color))?;
    Ok(brush)
}

/// Escapes text for use inside a XAML attribute / element value.
pub fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

pub fn ui(e: &impl Interface) -> Result<UIElement> {
    e.cast()
}

pub fn fe(e: &impl Interface) -> Result<FrameworkElement> {
    e.cast()
}

pub fn visibility(shown: bool) -> Visibility {
    if shown { Visibility::Visible } else { Visibility::Collapsed }
}

/// COM identity: two interface pointers denote the same object iff their IUnknown pointers are equal.
pub fn same(a: &impl Interface, b: &impl Interface) -> bool {
    match (a.cast::<windows_core::IUnknown>(), b.cast::<windows_core::IUnknown>()) {
        (Ok(a), Ok(b)) => a.as_raw() == b.as_raw(),
        _ => false,
    }
}

/// Minimal-mode bindings only expose the *default* interface of a class; these helpers cast to the interface
/// that owns a frequently used member.
pub trait Ext: Interface + Sized {
    fn xaml_root(&self) -> Result<XamlRoot> {
        self.cast::<UIElement>()?.XamlRoot()
    }
    fn items(&self) -> Result<ItemCollection> {
        self.cast::<ItemsControl>()?.Items()
    }
    fn selected_index(&self) -> Result<i32> {
        self.cast::<Selector>()?.SelectedIndex()
    }
    fn set_selected_index(&self, index: i32) -> Result<()> {
        self.cast::<Selector>()?.SetSelectedIndex(index)
    }
    fn on_click<F>(&self, f: F) -> Result<EventRevoker>
    where
        F: Fn(windows_core::Ref<windows_core::IInspectable>, windows_core::Ref<RoutedEventArgs>) + 'static,
    {
        self.cast::<ButtonBase>()?.Click(f)
    }
}
impl<T: Interface> Ext for T {}

pub fn app_window(window: &Window) -> Result<AppWindow> {
    window.cast::<IWindow2>()?.AppWindow()
}

/// Wraps the body of a WinRT event handler in [`guard`]: `cb!("name", |sender, args| { ... })` is the closure
/// the binding wants, with its panics logged instead of aborting the process.
macro_rules! cb {
    ($name:literal, |$($arg:pat_param),*| $body:expr) => {
        move |$($arg),*| $crate::util::guard($name, || $body)
    };
}

/// Runs a UI callback so that a panic is logged instead of unwinding into the WinRT event dispatcher, which
/// would abort the process (an `extern "system"` frame cannot be unwound through).
pub fn guard(what: &str, f: impl FnOnce()) {
    if let Err(panic) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        let message = panic
            .downcast_ref::<&str>()
            .map(|s| (*s).to_owned())
            .or_else(|| panic.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "panic".to_owned());
        log(&format!("PANIC in {what}: {message}"));
    }
}
