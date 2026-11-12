//! Pseudo-terminal hosting for Nekominal: process creation inside a ConPTY on Windows and a pty on Unix, ordered and
//! back-pressured output delivery, and discovery of the shells a user can launch.

#[cfg(windows)]
pub mod conpty;
#[cfg(unix)]
#[path = "unix.rs"]
pub mod conpty;
#[cfg(windows)]
pub mod handoff;
#[cfg(windows)]
pub mod netdrive;
pub mod profiles;
#[cfg(windows)]
pub mod relocate;
pub mod session;
#[cfg(windows)]
pub mod sudo;
