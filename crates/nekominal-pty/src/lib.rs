//! ConPTY hosting for Nekominal: process creation inside a pseudoconsole, ordered and
//! back-pressured output delivery, and discovery of the shells a user can launch.

pub mod conpty;
pub mod handoff;
pub mod netdrive;
pub mod profiles;
pub mod relocate;
pub mod session;
