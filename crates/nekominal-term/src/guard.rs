//! Bounds the memory an unterminated OSC string can take.
//!
//! vte (with its `std` feature, which alacritty_terminal enables) collects an OSC payload in a `Vec`
//! with no limit, and this crate runs two parsers over the stream. A program that writes `ESC ] 2 ;`
//! and then megabytes of text (a binary file `cat`-ed by mistake, or a hostile one) makes both grow
//! without bound: 64 MiB of payload cost 257 MiB of heap in the benchmark.
//!
//! `StringGuard` watches the stream for OSC strings with `memchr` (the ground state costs one SIMD
//! scan), and once a payload passes `MAX_OSC_PAYLOAD` it injects CAN so both parsers end the string
//! and drops the rest of it up to its terminator. vte dispatches the truncated prefix when CAN
//! arrives, so a payload above the cap is delivered cut at the cap.

use std::borrow::Cow;

use memchr::{memchr, memchr3};

/// Larger than any legitimate OSC (OSC 52 clipboard writes are the biggest in practice).
pub const MAX_OSC_PAYLOAD: usize = 4 * 1024 * 1024;

const ESC: u8 = 0x1b;
const CAN: u8 = 0x18;
const SUB: u8 = 0x1a;
const BEL: u8 = 0x07;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Ground,
    /// After ESC: the next byte decides.
    Escape,
    Osc,
    /// Dropping the rest of an over-long OSC string.
    Discard,
}

#[derive(Debug)]
pub struct StringGuard {
    state: State,
    payload: usize,
    limit: usize,
    /// Payload bytes dropped so far (for diagnostics).
    pub dropped: u64,
}

impl Default for StringGuard {
    fn default() -> Self {
        Self::with_limit(MAX_OSC_PAYLOAD)
    }
}

/// Position of the first byte that ends an OSC string: BEL, CAN, SUB or ESC.
fn find_terminator(bytes: &[u8]) -> Option<usize> {
    let a = memchr3(ESC, CAN, SUB, bytes);
    let b = memchr(BEL, bytes);
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

impl StringGuard {
    pub fn with_limit(limit: usize) -> Self {
        Self { state: State::Ground, payload: 0, limit, dropped: 0 }
    }

    /// Returns the bytes to feed the parsers: `bytes` itself unless a payload went over the limit.
    pub fn filter<'a>(&mut self, bytes: &'a [u8]) -> Cow<'a, [u8]> {
        let mut out: Vec<u8> = Vec::new();
        let mut modified = false;
        let mut copy_from = 0;
        let mut i = 0;

        while i < bytes.len() {
            match self.state {
                State::Ground => match memchr(ESC, &bytes[i..]) {
                    None => break,
                    Some(at) => {
                        i += at + 1;
                        self.state = State::Escape;
                    }
                },
                State::Escape => {
                    match bytes[i] {
                        b']' => {
                            self.state = State::Osc;
                            self.payload = 0;
                        }
                        ESC => {}
                        _ => self.state = State::Ground,
                    }
                    i += 1;
                }
                State::Osc => {
                    let rest = &bytes[i..];
                    let end = find_terminator(rest);
                    let segment = end.unwrap_or(rest.len());
                    let room = self.limit - self.payload;
                    if segment > room {
                        // Cut here: keep `room` bytes, end the string with CAN, drop the remainder.
                        out.extend_from_slice(&bytes[copy_from..i + room]);
                        out.push(CAN);
                        copy_from = i + room;
                        modified = true;
                        i += room;
                        self.state = State::Discard;
                    } else {
                        self.payload += segment;
                        i += segment;
                        if end.is_some() {
                            // BEL / CAN / SUB end the string; ESC starts the ST (or another sequence).
                            self.state = if bytes[i] == ESC { State::Escape } else { State::Ground };
                            i += 1;
                        }
                    }
                }
                State::Discard => {
                    let rest = &bytes[i..];
                    let end = find_terminator(rest);
                    let segment = end.unwrap_or(rest.len());
                    self.dropped += segment as u64;
                    // Drop [i, i + segment); keep an ESC terminator, drop BEL / CAN / SUB.
                    out.extend_from_slice(&bytes[copy_from..i]);
                    modified = true;
                    i += segment;
                    copy_from = i;
                    if end.is_some() {
                        if bytes[i] == ESC {
                            self.state = State::Escape;
                        } else {
                            self.state = State::Ground;
                            copy_from = i + 1;
                        }
                        i += 1;
                    }
                }
            }
        }

        if modified {
            out.extend_from_slice(&bytes[copy_from..]);
            Cow::Owned(out)
        } else {
            Cow::Borrowed(bytes)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_streams_are_passed_through_untouched() {
        let mut guard = StringGuard::with_limit(16);
        let data = b"hello \x1b[31mred\x1b[0m \x1b]2;title\x07 and \x1b]7;file:///x\x1b\\ done";
        assert!(matches!(guard.filter(data), Cow::Borrowed(_)));
    }

    #[test]
    fn an_over_long_osc_is_cut_with_can_and_its_rest_is_dropped() {
        let mut guard = StringGuard::with_limit(8);
        let mut data = b"a\x1b]2;".to_vec();
        data.extend(std::iter::repeat_n(b'x', 100));
        data.extend_from_slice(b"\x07b");
        let out = guard.filter(&data);
        assert_eq!(&*out, b"a\x1b]2;xxxxxx\x18b"); // the limit counts the "2;" too
        assert_eq!(guard.dropped, 94);
    }

    #[test]
    fn the_count_continues_across_chunks_and_an_esc_terminator_is_kept() {
        let mut guard = StringGuard::with_limit(8);
        let mut stream = Vec::new();
        stream.extend_from_slice(&guard.filter(b"\x1b]0;aaaa"));
        stream.extend_from_slice(&guard.filter(b"bbbbcccc"));
        stream.extend_from_slice(&guard.filter(b"dddd\x1b\\tail"));
        assert_eq!(stream, b"\x1b]0;aaaabb\x18\x1b\\tail");
    }

    #[test]
    fn short_strings_after_a_cut_work_again() {
        let mut guard = StringGuard::with_limit(4);
        let out = guard.filter(b"\x1b]2;toolongpayload\x07\x1b]2;ok\x07z").into_owned();
        assert_eq!(out, b"\x1b]2;to\x18\x1b]2;ok\x07z");
    }

    #[test]
    fn escape_sequences_that_are_not_osc_do_not_count() {
        let mut guard = StringGuard::with_limit(4);
        let data = b"\x1b[38;2;1;2;3mXXXXXXXXXXXXXXXXXXXX\x1bPq0123456789\x1b\\\x1b_Gabcdefghijkl\x1b\\";
        assert!(matches!(guard.filter(data), Cow::Borrowed(_)));
    }
}
