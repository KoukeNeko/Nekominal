//! Whether the window may be translucent. A compositor blends the alpha of a window with what is behind it whatever the
//! theme of the system says, so on Linux the answer is that it may.

pub fn on() -> bool {
    true
}

pub fn refresh() {}
