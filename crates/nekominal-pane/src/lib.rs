//! Toolkit-free pane host.
//!
//! * [`host::Pane`]: one ConPTY session, one `TerminalCore`, the single PTY writer thread, key/text/paste input
//!   encoded for the program's current modes.
//! * [`view`]: `Snapshot` -> `nekominal_render::GridView` adapter (colours stay symbolic).
//! * [`draw::PaneRenderer`]: drives a `nekominal_render::Renderer` for one pane (snapshot, update, draw) on
//!   whatever render target the shell provides; usable headless.

pub mod draw;
pub mod hints;
pub mod host;
pub mod pointer;
pub mod view;

pub use draw::PaneRenderer;
pub use host::{ClipboardRequest, MAX_CLIPBOARD_TEXT, Pane, UiEvent, Wake, input_modes_of};
pub use pointer::{Pointer, PointerEffects};
