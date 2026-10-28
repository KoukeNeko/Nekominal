//! Nekominal emulator spike: alacritty_terminal as the VT core, driven from the PTY forwarder thread,
//! plus a chunk-boundary-safe sniffer for the sequences alacritty_terminal does not surface.

pub mod agent_status;
mod graphemes;
pub mod guard;
pub mod images;
pub mod kitty;
pub mod links;
pub mod terminal;
pub mod osc;
pub mod restore;
pub mod sixel;
pub mod sniffer;
pub mod theme;
pub mod triggers;

pub use alacritty_terminal;
pub use crate::terminal::{
    CellColor, CellSide, ClipboardFormatter, ClipboardKind, CoreConfig, CoreEvent, Damage, LineDamage, LineId,
    CommandOutput, MarkCursor, MouseExtensions, SearchMatch, SearchSession, SelectionKind, SnapCell, SnapCursor, Snapshot, StyledRun,
    TermDims, TerminalCore, TriggerMark, DA1_WITH_SIXEL, DEFAULT_SCROLLBACK_BYTES, SEMANTIC_ESCAPE_CHARS_CJK, scrollback_bytes_for, sticky_prompt_line,
};
pub use images::SnapImage;
pub use links::{Link, LinkKind, OpenTarget, Refusal};
pub use sniffer::{MouseExtension, SixelMode, Sniffed, Sniffer, Unsupported};
pub use theme::Palette;
