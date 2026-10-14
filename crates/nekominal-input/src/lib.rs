//! Terminal input encoding as a pure-logic library: keyboard, mouse, focus and paste events in,
//! the bytes a terminal application expects out. No windowing dependency; the UI toolkit fills
//! [`KeyInput`] / [`MouseInput`] and writes the returned bytes to the PTY.
//!
//! [`KeyAssembler`] and [`MouseAssembler`] sit on top of the encoders and take the raw event
//! streams of the toolkit (WinUI 3 key and pointer events), so the window code only forwards events.

pub mod key;
pub mod key_assembler;
pub mod keyboard;
pub mod modes;
pub mod mouse;
pub mod mouse_assembler;
pub mod paste;
pub mod scancode;
pub mod utf16;
pub mod vk;
pub mod win32;

pub use key::{
    is_altgr, Key, KeyEvent, KeyInput, KeypadKey, MediaKey, ModifierKey, Mods,
};
pub use key_assembler::{KeyAssembler, KeyOutput, KeyStroke, KeyboardEvent, ALTGR_WINDOW_MS};
pub use keyboard::{encode_key, encode_text};
pub use modes::{InputModes, KittyFlags, MouseEncoding, MouseMode};
pub use mouse::{
    encode_mouse, MouseButton, MouseInput, MouseKind, MouseState, WheelAccumulator, WheelDirection,
};
pub use mouse_assembler::{
    cell_at, CellPoint, MouseAssembler, PointerConfig, PointerEvent, PointerGrid, PointerKind, PointerOutcome,
    SelectionKind, Side,
};
pub use paste::{encode_focus, prepare_paste};
pub use scancode::{physical_key, PhysicalKey};
pub use utf16::Utf16Assembler;
pub use win32::{
    conpty_route, encode_key_conpty, encode_win32_input, encode_win32_text, ConptyRoute, Win32Key,
};
