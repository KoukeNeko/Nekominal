//! A session handed to another process keeps running there: the program is never stopped, what it wrote while nobody read
//! comes through, and input, resize and exit work on the side that took it.
//!
//! As in an update, the process that gives the session exits without closing anything (`TerminateProcess` on itself:
//! no destructor, no `ClosePseudoConsole`), after duplicating the handles into the process that takes it. That needs two
//! processes, so this test has no harness: it runs itself again as the child that starts the session and gives it away.

#[cfg(windows)]
include!("handoff/windows.rs");

#[cfg(not(windows))]
fn main() {}
