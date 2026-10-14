//! Mouse reports (DECSET 9 / 1000 / 1002 / 1003 with the default, UTF-8, SGR, URXVT and
//! SGR-pixels encodings) and alternate scroll (DECSET 1007).

use crate::{
    key::Mods,
    modes::{InputModes, MouseEncoding, MouseMode},
};

const ESC: u8 = 0x1b;

/// Largest position the single-byte default encoding can carry (255 - 32).
pub const DEFAULT_ENCODING_MAX: u32 = 223;
/// Most wheel notches reported for one event; a runaway delta must not become megabytes of input.
pub const MAX_NOTCHES: usize = 64;

/// Largest position that stays 7-bit ASCII in the default encoding (127 - 32). ConPTY reads its
/// input pipe as UTF-8 text, so a byte above 0x7f in the default encoding arrives as U+FFFD (measured
/// against the bundled OpenConsole 1.25, see the blueprint). Used when `pty_decodes_utf8` is set.
pub const DEFAULT_ENCODING_ASCII_MAX: u32 = 95;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    /// Button 8 (browser back, XButton1).
    Back,
    /// Button 9 (browser forward, XButton2).
    Forward,
}

impl MouseButton {
    fn code(self) -> u32 {
        match self {
            MouseButton::Left => 0,
            MouseButton::Middle => 1,
            MouseButton::Right => 2,
            MouseButton::Back => 128,
            MouseButton::Forward => 129,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WheelDirection {
    Up,
    Down,
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseKind {
    Press(MouseButton),
    Release(MouseButton),
    /// Pointer motion; `held` is the button that is down, if any.
    Move { held: Option<MouseButton> },
    /// One or more wheel notches ([`MouseInput::notches`]).
    Wheel(WheelDirection),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MouseInput {
    pub kind: MouseKind,
    pub mods: Mods,
    /// Zero-based cell position.
    pub col: u16,
    pub row: u16,
    /// Zero-based position in pixels inside the terminal grid, for SGR-pixels (1016).
    pub pixel: Option<(u32, u32)>,
    /// Wheel notches to report (at least 1); ignored for other events.
    pub notches: u16,
}

impl MouseInput {
    pub fn new(kind: MouseKind, col: u16, row: u16) -> Self {
        MouseInput { kind, mods: Mods::NONE, col, row, pixel: None, notches: 1 }
    }

    pub fn with_mods(mut self, mods: Mods) -> Self {
        self.mods = mods;
        self
    }

    pub fn with_pixel(mut self, x: u32, y: u32) -> Self {
        self.pixel = Some((x, y));
        self
    }

    pub fn with_notches(mut self, notches: u16) -> Self {
        self.notches = notches.max(1);
        self
    }
}

/// Per-pane memory: motion is reported only when the position changed since the last report.
#[derive(Clone, Debug, Default)]
pub struct MouseState {
    last: Option<(u32, u32)>,
}

impl MouseState {
    /// Forget the last position, for example when the mouse mode changes.
    pub fn reset(&mut self) {
        self.last = None;
    }
}

/// Turns wheel deltas into whole notches. Windows reports 120 per notch for a classic wheel and
/// fractions of it for precision touchpads; sending a report per event would scroll far too
/// fast, dropping the remainder would lose slow scrolls.
#[derive(Clone, Debug, Default)]
pub struct WheelAccumulator {
    remainder: i32,
}

impl WheelAccumulator {
    pub const NOTCH: i32 = 120;

    /// Adds a delta (positive: away from the user / up) and returns the whole notches now due,
    /// signed like the delta.
    pub fn push(&mut self, delta: i32) -> i32 {
        if (delta < 0) != (self.remainder < 0) && delta != 0 {
            self.remainder = 0;
        }
        self.remainder += delta;
        let notches = self.remainder / Self::NOTCH;
        self.remainder -= notches * Self::NOTCH;
        notches
    }
}

fn arrow(direction: WheelDirection, app_cursor: bool) -> [u8; 3] {
    let final_byte = match direction {
        WheelDirection::Up => b'A',
        WheelDirection::Down => b'B',
        WheelDirection::Right => b'C',
        WheelDirection::Left => b'D',
    };
    [ESC, if app_cursor { b'O' } else { b'[' }, final_byte]
}

fn button_field(input: &MouseInput, encoding: MouseEncoding) -> Option<u32> {
    let sgr_like = matches!(encoding, MouseEncoding::Sgr | MouseEncoding::SgrPixels);
    let mut code = match input.kind {
        MouseKind::Press(button) => button.code(),
        // The older encodings cannot say which button went up.
        MouseKind::Release(button) => {
            if sgr_like {
                button.code()
            } else {
                3
            }
        }
        MouseKind::Move { held } => 32 + held.map_or(3, MouseButton::code),
        MouseKind::Wheel(direction) => {
            64 + match direction {
                WheelDirection::Up => 0,
                WheelDirection::Down => 1,
                WheelDirection::Left => 2,
                WheelDirection::Right => 3,
            }
        }
    };
    if input.mods.contains(Mods::SHIFT) {
        code += 4;
    }
    if input.mods.intersects(Mods::ALT | Mods::META) {
        code += 8;
    }
    if input.mods.contains(Mods::CTRL) {
        code += 16;
    }
    Some(code)
}

fn push_utf8(out: &mut Vec<u8>, value: u32) -> Option<()> {
    let ch = char::from_u32(value)?;
    let mut buf = [0u8; 4];
    out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
    Some(())
}

fn serialize(
    code: u32,
    x: u32,
    y: u32,
    release: bool,
    encoding: MouseEncoding,
    default_max: u32,
) -> Option<Vec<u8>> {
    match encoding {
        MouseEncoding::Default => {
            if x > default_max || y > default_max || code > default_max {
                return None;
            }
            Some(vec![ESC, b'[', b'M', (code + 32) as u8, (x + 32) as u8, (y + 32) as u8])
        }
        MouseEncoding::Utf8 => {
            // Cb, Cx and Cy are UTF-8 encoded characters (up to U+07FF: position 2015).
            let mut out = vec![ESC, b'[', b'M'];
            for value in [code + 32, x + 32, y + 32] {
                if value > 0x7ff {
                    return None;
                }
                push_utf8(&mut out, value)?;
            }
            Some(out)
        }
        MouseEncoding::Sgr | MouseEncoding::SgrPixels => {
            let final_byte = if release { 'm' } else { 'M' };
            Some(format!("\x1b[<{code};{x};{y}{final_byte}").into_bytes())
        }
        MouseEncoding::Urxvt => Some(format!("\x1b[{};{x};{y}M", code + 32).into_bytes()),
    }
}

/// Translates one mouse event.
///
/// `None` when the event is not reported: no tracking mode, the mode does not include this kind
/// of event, motion that stayed in the same cell, or a position the encoding cannot carry. A
/// wheel event with no tracking mode is turned into cursor keys on the alternate screen when
/// alternate scroll is on; otherwise the UI scrolls the scrollback itself.
///
/// The caller decides before this whether the event belongs to the application at all (the usual
/// rule: Shift held means the terminal handles it).
pub fn encode_mouse(input: &MouseInput, modes: &InputModes, state: &mut MouseState) -> Option<Vec<u8>> {
    if let MouseKind::Wheel(direction) = input.kind
        && modes.mouse_mode == MouseMode::None
    {
        if !(modes.alt_screen && modes.alternate_scroll) {
            return None;
        }
        let arrow = arrow(direction, modes.app_cursor);
        return Some(arrow.repeat(usize::from(input.notches.max(1)).min(MAX_NOTCHES)));
    }

    let tracked = match (modes.mouse_mode, input.kind) {
        (MouseMode::None, _) => false,
        (MouseMode::X10, MouseKind::Press(b)) => {
            matches!(b, MouseButton::Left | MouseButton::Middle | MouseButton::Right)
        }
        (MouseMode::X10, _) => false,
        (_, MouseKind::Press(_) | MouseKind::Release(_) | MouseKind::Wheel(_)) => true,
        (MouseMode::Normal, MouseKind::Move { .. }) => false,
        (MouseMode::Button, MouseKind::Move { held }) => held.is_some(),
        (MouseMode::Any, MouseKind::Move { .. }) => true,
    };
    if !tracked {
        return None;
    }

    let (x, y) = if modes.mouse_encoding == MouseEncoding::SgrPixels {
        input.pixel?
    } else {
        (u32::from(input.col) + 1, u32::from(input.row) + 1)
    };

    if matches!(input.kind, MouseKind::Move { .. }) && state.last == Some((x, y)) {
        return None;
    }

    // X10 reports only the button, never modifiers.
    let mut code = button_field(input, modes.mouse_encoding)?;
    if modes.mouse_mode == MouseMode::X10 {
        code = match input.kind {
            MouseKind::Press(b) => b.code(),
            _ => return None,
        };
    }

    let release = matches!(input.kind, MouseKind::Release(_));
    let notches = match input.kind {
        MouseKind::Wheel(_) => usize::from(input.notches.max(1)).min(MAX_NOTCHES),
        _ => 1,
    };
    let default_max = if modes.pty_decodes_utf8 { DEFAULT_ENCODING_ASCII_MAX } else { DEFAULT_ENCODING_MAX };
    let report = serialize(code, x, y, release, modes.mouse_encoding, default_max)?;
    if !matches!(input.kind, MouseKind::Wheel(_)) {
        state.last = Some((x, y));
    }
    Some(report.repeat(notches))
}
