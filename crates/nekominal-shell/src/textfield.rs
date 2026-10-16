//! A line of text with a caret and a selection: what the query of the palette and of the find bar, the search of the settings
//! page and the fields of its rows are edited with. The text is a `String`; the caret and the selection count characters, so
//! that a character outside the Basic Multilingual Plane is one step. Pure.

use nekominal_input::Utf16Assembler;

/// Where the caret is, and where the selection started when there is one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cursor {
    pub caret: usize,
    pub anchor: Option<usize>,
}

impl Cursor {
    pub fn at_end(text: &str) -> Cursor {
        Cursor { caret: length(text), anchor: None }
    }

    /// The selected characters, as the first and the one after the last; `None` for an empty selection.
    pub fn selection(&self) -> Option<(usize, usize)> {
        self.anchor.filter(|anchor| *anchor != self.caret).map(|anchor| (anchor.min(self.caret), anchor.max(self.caret)))
    }
}

pub fn length(text: &str) -> usize {
    text.chars().count()
}

fn byte(text: &str, characters: usize) -> usize {
    text.char_indices().nth(characters).map_or(text.len(), |(at, _)| at)
}

/// The text before the `characters`-th character.
pub fn prefix(text: &str, characters: usize) -> &str {
    &text[..byte(text, characters)]
}

/// Takes the selection out. False when there was none.
pub fn delete_selection(text: &mut String, cursor: &mut Cursor) -> bool {
    let selection = cursor.selection();
    cursor.anchor = None;
    let Some((from, to)) = selection else { return false };
    text.replace_range(byte(text, from)..byte(text, to), "");
    cursor.caret = from;
    true
}

/// Puts `inserted` at the caret, in the place of the selection.
pub fn insert(text: &mut String, cursor: &mut Cursor, inserted: &str) {
    delete_selection(text, cursor);
    let at = byte(text, cursor.caret);
    text.insert_str(at, inserted);
    cursor.caret += length(inserted);
}

pub fn backspace(text: &mut String, cursor: &mut Cursor) -> bool {
    if delete_selection(text, cursor) {
        return true;
    }
    if cursor.caret == 0 {
        return false;
    }
    let (from, to) = (byte(text, cursor.caret - 1), byte(text, cursor.caret));
    text.replace_range(from..to, "");
    cursor.caret -= 1;
    true
}

pub fn delete(text: &mut String, cursor: &mut Cursor) -> bool {
    if delete_selection(text, cursor) {
        return true;
    }
    if cursor.caret >= length(text) {
        return false;
    }
    let (from, to) = (byte(text, cursor.caret), byte(text, cursor.caret + 1));
    text.replace_range(from..to, "");
    true
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion {
    Left,
    Right,
    Home,
    End,
    WordLeft,
    WordRight,
}

/// The start of the word before `at`: spaces are skipped, then the letters.
fn word_start(text: &str, at: usize) -> usize {
    let chars: Vec<char> = text.chars().collect();
    let mut i = at.min(chars.len());
    while i > 0 && chars[i - 1].is_whitespace() {
        i -= 1;
    }
    while i > 0 && !chars[i - 1].is_whitespace() {
        i -= 1;
    }
    i
}

/// The end of the word after `at`.
fn word_end(text: &str, at: usize) -> usize {
    let chars: Vec<char> = text.chars().collect();
    let mut i = at.min(chars.len());
    while i < chars.len() && chars[i].is_whitespace() {
        i += 1;
    }
    while i < chars.len() && !chars[i].is_whitespace() {
        i += 1;
    }
    i
}

/// Moves the caret. With `extend` the selection grows to it; without, Left and Right with a selection go to its edge and
/// everything else drops the selection.
pub fn move_caret(text: &str, cursor: &mut Cursor, motion: Motion, extend: bool) {
    if !extend
        && let Some((from, to)) = cursor.selection()
        && matches!(motion, Motion::Left | Motion::Right)
    {
        cursor.caret = if motion == Motion::Left { from } else { to };
        cursor.anchor = None;
        return;
    }
    let last = length(text);
    let target = match motion {
        Motion::Left => cursor.caret.saturating_sub(1),
        Motion::Right => (cursor.caret + 1).min(last),
        Motion::Home => 0,
        Motion::End => last,
        Motion::WordLeft => word_start(text, cursor.caret),
        Motion::WordRight => word_end(text, cursor.caret),
    };
    cursor.anchor = if extend { cursor.anchor.or(Some(cursor.caret)) } else { None };
    cursor.caret = target;
}

pub fn select_all(text: &str, cursor: &mut Cursor) {
    cursor.anchor = Some(0);
    cursor.caret = length(text);
}

pub fn selected(text: &str, cursor: &Cursor) -> Option<String> {
    cursor.selection().map(|(from, to)| text[byte(text, from)..byte(text, to)].to_owned())
}

/// One UTF-16 unit typed: a character goes in at the caret, Backspace takes one out, the halves of a surrogate pair wait for
/// each other. True when the text changed.
pub fn type_unit(text: &mut String, cursor: &mut Cursor, input: &mut Utf16Assembler, unit: u16) -> bool {
    match unit {
        0x08 => {
            *input = Utf16Assembler::default();
            backspace(text, cursor)
        }
        u if u >= 0x20 && u != 0x7F => match input.push(u) {
            Some(ch) => {
                insert(text, cursor, ch.encode_utf8(&mut [0; 4]));
                true
            }
            None => false,
        },
        _ => false,
    }
}

/// What a key did to the line.
#[derive(Debug, PartialEq, Eq)]
pub enum Edit {
    /// Not a key of a line of text.
    Ignored,
    /// The caret or the selection moved, or the text changed (`changed`).
    Done { changed: bool },
    /// The selection is to be put on the clipboard; for a cut it is out of the text already.
    Copy(String),
    Cut(String),
}

/// A key of a line of text: the arrows, Home and End (with Ctrl by words, with Shift selecting), Delete, and Ctrl+A, Ctrl+C
/// and Ctrl+X.
pub fn key(text: &mut String, cursor: &mut Cursor, vk_code: i32, ctrl: bool, shift: bool) -> Edit {
    let motion = match vk_code {
        0x25 => Some(if ctrl { Motion::WordLeft } else { Motion::Left }),
        0x27 => Some(if ctrl { Motion::WordRight } else { Motion::Right }),
        0x24 => Some(Motion::Home),
        0x23 => Some(Motion::End),
        _ => None,
    };
    if let Some(motion) = motion {
        move_caret(text, cursor, motion, shift);
        return Edit::Done { changed: false };
    }
    match (vk_code, ctrl) {
        (0x2E, false) => Edit::Done { changed: delete(text, cursor) },
        (0x41, true) => {
            select_all(text, cursor);
            Edit::Done { changed: false }
        }
        (0x43, true) => selected(text, cursor).map_or(Edit::Done { changed: false }, Edit::Copy),
        (0x58, true) => match selected(text, cursor) {
            Some(part) => {
                delete_selection(text, cursor);
                Edit::Cut(part)
            }
            None => Edit::Done { changed: false },
        },
        _ => Edit::Ignored,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(text: &str) -> (String, Cursor) {
        (text.to_owned(), Cursor::at_end(text))
    }

    #[test]
    fn typing_goes_in_at_the_caret_and_replaces_the_selection() {
        let (mut text, mut cursor) = field("abcd");
        cursor.caret = 2;
        insert(&mut text, &mut cursor, "XY");
        assert_eq!((text.as_str(), cursor.caret), ("abXYcd", 4));
        cursor.anchor = Some(1);
        insert(&mut text, &mut cursor, "-");
        assert_eq!((text.as_str(), cursor.caret, cursor.anchor), ("a-cd", 2, None));
    }

    #[test]
    fn backspace_and_delete_take_one_character_or_the_selection() {
        let (mut text, mut cursor) = field("a你好b");
        cursor.caret = 2;
        assert!(backspace(&mut text, &mut cursor));
        assert_eq!((text.as_str(), cursor.caret), ("a好b", 1));
        assert!(delete(&mut text, &mut cursor));
        assert_eq!(text, "ab");
        cursor = Cursor { caret: 0, anchor: None };
        assert!(!backspace(&mut text, &mut cursor), "nothing before the first character");
        cursor.caret = 2;
        assert!(!delete(&mut text, &mut cursor), "nothing after the last one");
        cursor = Cursor { caret: 2, anchor: Some(0) };
        assert!(backspace(&mut text, &mut cursor));
        assert_eq!((text.as_str(), cursor.caret), ("", 0));
    }

    #[test]
    fn a_character_outside_the_basic_plane_is_one_step() {
        let (mut text, mut cursor) = field("a😀b");
        move_caret(&text, &mut cursor, Motion::Left, false);
        move_caret(&text, &mut cursor, Motion::Left, false);
        assert_eq!(cursor.caret, 1);
        assert!(delete(&mut text, &mut cursor));
        assert_eq!(text, "ab");
        let mut input = Utf16Assembler::default();
        let (mut typed, mut at) = (String::new(), Cursor::default());
        assert!(!type_unit(&mut typed, &mut at, &mut input, 0xD83D), "half of a pair changes nothing");
        assert!(type_unit(&mut typed, &mut at, &mut input, 0xDE00));
        assert_eq!((typed.as_str(), at.caret), ("😀", 1));
        assert!(type_unit(&mut typed, &mut at, &mut input, 8));
        assert_eq!(typed, "");
        assert!(!type_unit(&mut typed, &mut at, &mut input, 13), "Enter is not text");
    }

    #[test]
    fn shift_grows_the_selection_and_a_plain_arrow_goes_to_its_edge() {
        let (text, mut cursor) = field("hello world");
        cursor.caret = 5;
        move_caret(&text, &mut cursor, Motion::Right, true);
        move_caret(&text, &mut cursor, Motion::Right, true);
        assert_eq!(cursor.selection(), Some((5, 7)));
        move_caret(&text, &mut cursor, Motion::Left, false);
        assert_eq!((cursor.caret, cursor.selection()), (5, None));
        move_caret(&text, &mut cursor, Motion::End, true);
        assert_eq!(cursor.selection(), Some((5, 11)));
        move_caret(&text, &mut cursor, Motion::Home, false);
        assert_eq!((cursor.caret, cursor.anchor), (0, None));
    }

    #[test]
    fn ctrl_moves_by_words() {
        let (text, mut cursor) = field("one  two three");
        move_caret(&text, &mut cursor, Motion::WordLeft, false);
        assert_eq!(cursor.caret, 9);
        move_caret(&text, &mut cursor, Motion::WordLeft, false);
        assert_eq!(cursor.caret, 5);
        move_caret(&text, &mut cursor, Motion::WordRight, false);
        assert_eq!(cursor.caret, 8);
        cursor.caret = 0;
        move_caret(&text, &mut cursor, Motion::WordRight, false);
        assert_eq!(cursor.caret, 3);
    }

    #[test]
    fn the_keys_of_a_line_select_copy_and_cut() {
        let (mut text, mut cursor) = field("alpha beta");
        assert_eq!(key(&mut text, &mut cursor, 0x41, true, false), Edit::Done { changed: false });
        assert_eq!(cursor.selection(), Some((0, 10)));
        assert_eq!(key(&mut text, &mut cursor, 0x43, true, false), Edit::Copy("alpha beta".into()));
        assert_eq!(text, "alpha beta");
        cursor = Cursor { caret: 5, anchor: Some(0) };
        assert_eq!(key(&mut text, &mut cursor, 0x58, true, false), Edit::Cut("alpha".into()));
        assert_eq!((text.as_str(), cursor.caret), (" beta", 0));
        assert_eq!(key(&mut text, &mut cursor, 0x2E, false, false), Edit::Done { changed: true });
        assert_eq!(text, "beta");
        assert_eq!(key(&mut text, &mut cursor, 0x41 + 1, false, false), Edit::Ignored);
        assert_eq!(key(&mut text, &mut cursor, 0x56, true, false), Edit::Ignored, "paste is the application's");
    }
}
