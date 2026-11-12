//! Closing a session from a thread-local destructor while the process exits must neither panic
//! nor hang.
//!
//! During exit the OS refuses to create threads (`ERROR_ACCESS_DENIED`). `Sessions::close` used to
//! `thread::spawn` and panicked when a pane was dropped by a main-thread TLS destructor. That
//! needs a real process exit, so this test has no harness: it runs itself again as a child that
//! returns from `main` with a live session held by a TLS destructor.

#[cfg(windows)]
include!("close_at_exit/windows.rs");

#[cfg(not(windows))]
fn main() {}
