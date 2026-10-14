//! The one audible path of the application: the bell of a program, when `bell.sound` asks for it. Nothing else in
//! the interface makes a sound, and the silence test (`tests/silence.rs`) allows sound APIs in this file only.

use windows::core::BOOL;

use crate::util::log;

// Declared here: the `windows` crate keeps it behind the whole `Win32_System_Diagnostics_Debug` feature.
#[link(name = "user32")]
unsafe extern "system" {
    fn MessageBeep(sound: u32) -> BOOL;
}

/// `MB_OK`: the default system sound.
const MB_OK: u32 = 0;

/// Plays the default system sound; it returns at once.
pub fn bell() {
    // SAFETY: a plain call with a defined sound type; it touches no memory of ours.
    if !unsafe { MessageBeep(MB_OK) }.as_bool() {
        log("bell sound: MessageBeep failed");
    }
}
