//! Headless harness for driving coding-agent CLIs through ConPTY and the emulator.
//!
//! * [`probe::Probe`] runs a program in a ConPTY session into a `TerminalCore`, answers its queries like the
//!   pane host does, and lets a test type, paste, click, resize and read the screen, modes and events.
//! * [`inventory::Inventory`] counts every control sequence the program emitted and whether it was answered.
//! * [`env`] builds the program's environment from an allow-list; [`scrub`] removes personal data from text
//!   that leaves `target/agent-probe/`.
//! * [`claude`] holds what is specific to Claude Code: where it is, which slash commands are local, and how
//!   to read its screens.
//!
//! `tests/selftest.rs` checks the harness with `cmd.exe` and PowerShell (no CLI, no network).
//! `tests/claude_code.rs` runs the installed Claude Code and is ignored by default; its header says how to run it.

pub mod claude;
pub mod env;
pub mod inventory;
pub mod keys;
pub mod probe;
pub mod screen;
pub mod scrub;
pub mod sys;

pub use keys::KeySpec;
pub use probe::{Guard, GuardStats, KeyRouting, Modes, Observed, Probe, ProbeError, SpawnOptions};
pub use screen::{Diff, Screen};
