//! Mouse cursor without subclassing UIElement.
//!
//! `UIElement.ProtectedCursor` is a protected property, only reachable from a derived class. The public way to
//! change the cursor from Rust is the island-level `InputPointerSource.Cursor` of the `ContentIsland` that hosts
//! the window content (`XamlRoot.ContentIsland`). XAML puts the arrow back on every pointer move, so a handler
//! that wants another cursor has to set it on every move, not only on enter.

use nekominal_winui_sys::*;
use windows_core::{Interface, Result};

fn pointer_source(el: &UIElement) -> Result<InputPointerSource> {
    let island = el.XamlRoot()?.cast::<IXamlRoot4>()?.ContentIsland()?;
    InputPointerSource::GetForIsland(&island)
}

pub fn set_cursor(el: &UIElement, shape: InputSystemCursorShape) -> Result<()> {
    let cursor: InputCursor = InputSystemCursor::Create(shape)?.cast()?;
    pointer_source(el)?.SetCursor(&cursor)
}
