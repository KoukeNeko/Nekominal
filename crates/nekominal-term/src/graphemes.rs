//! Grapheme clusters for alacritty_terminal, which gives every code point its own width: a ZWJ family is three
//! wide glyphs there, one in Claude Code (string-width) and in ConPTY's grapheme mode. The text that follows
//! then lands up to four columns right of where the program put it, wraps, and shifts the whole frame.
//!
//! `Clusters` stands between the parser and `Term`: a character that continues the cluster of the cell just
//! written (after a ZWJ, a skin tone, the second regional indicator, VS16, a combining mark) is stored in that
//! cell, and the cell becomes wide when the cluster does (`❤` + VS16). Everything else goes to `Term` as is.

use alacritty_terminal::{
    event::EventListener,
    grid::Dimensions,
    term::{cell::Flags, Term},
    vte::ansi::{
        cursor_icon::CursorIcon, Attr, CharsetIndex, ClearMode, CursorShape, CursorStyle, Handler, Hyperlink,
        KeyboardModes, KeyboardModesApplyBehavior, LineClearMode, Mode, ModifyOtherKeys, PrivateMode, Rgb,
        ScpCharPath, ScpUpdateMode, StandardCharset, TabulationClearMode,
    },
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// `Term` seen through the cluster of the last printed cell, which `cluster` keeps between calls (and chunks).
pub struct Clusters<'a, T> {
    pub term: &'a mut Term<T>,
    pub cluster: &'a mut String,
}

impl<T: EventListener> Clusters<'_, T> {
    /// Adds `c` to the cell the cluster is in, widening it when the cluster became wide. False when that cell is
    /// not there any more (the program moved or wrote elsewhere in between).
    fn join(&mut self, c: char, joined: &str) -> bool {
        let columns = self.term.columns();
        let grid = self.term.grid_mut();
        let (line, mut column, wraps) = (grid.cursor.point.line, grid.cursor.point.column, grid.cursor.input_needs_wrap);
        if !wraps {
            if column.0 == 0 {
                return false;
            }
            column -= 1;
        }
        if grid[line][column].flags.contains(Flags::WIDE_CHAR_SPACER) {
            if column.0 == 0 {
                return false;
            }
            column -= 1;
        }
        if Some(grid[line][column].c) != joined.chars().next() {
            return false;
        }
        grid[line][column].push_zerowidth(c);
        let wide = grid[line][column].flags.contains(Flags::WIDE_CHAR);
        // shortcut: a cluster that became wide in the last column stays narrow; ConPTY would wrap it.
        // ConPTY's grapheme mode makes any character followed by VS16 two columns wide, emoji or not.
        if !wide && (joined.width() >= 2 || c == '\u{fe0f}') && !wraps && column.0 + 1 < columns {
            grid[line][column].flags.insert(Flags::WIDE_CHAR);
            let mut spacer = grid.cursor.template.clone();
            spacer.c = ' ';
            spacer.flags.insert(Flags::WIDE_CHAR_SPACER);
            grid[line][column + 1] = spacer;
            if column.0 + 2 < columns {
                grid.cursor.point.column = column + 2;
            } else {
                grid.cursor.point.column = column + 1;
                grid.cursor.input_needs_wrap = true;
            }
        }
        true
    }
}

/// Every other call ends the cluster and goes to `Term` unchanged.
macro_rules! forward {
    ($($name:ident($($arg:ident: $ty:ty),*);)*) => {
        $(fn $name(&mut self, $($arg: $ty),*) {
            self.cluster.clear();
            self.term.$name($($arg),*)
        })*
    };
}

/// The most bytes a cluster grows to; the longest real ones (a ZWJ family with skin tones) are about half this.
/// Joining scans the whole cluster, so without a cap a base and N combining marks cost N² under the terminal lock.
const MAX_CLUSTER_BYTES: usize = 64;

impl<T: EventListener> Handler for Clusters<'_, T> {
    fn input(&mut self, c: char) {
        if !self.cluster.is_empty() {
            // Past the cap a zero-width character is dropped: neither joined nor handed to `Term`, which would
            // stack it on the same cell.
            if self.cluster.len() > MAX_CLUSTER_BYTES && !c.width().is_some_and(|w| w > 0) {
                return;
            }
            let mut joined = std::mem::take(self.cluster);
            joined.push(c);
            if joined.graphemes(true).nth(1).is_none() && self.join(c, &joined) {
                *self.cluster = joined;
                return;
            }
        }
        if c.width().is_some_and(|w| w > 0) {
            self.cluster.push(c);
        }
        self.term.input(c);
    }

    forward! {
        set_title(a: Option<String>);
        set_cursor_style(a: Option<CursorStyle>);
        set_cursor_shape(a: CursorShape);
        goto(a: i32, b: usize);
        goto_line(a: i32);
        goto_col(a: usize);
        insert_blank(a: usize);
        move_up(a: usize);
        move_down(a: usize);
        identify_terminal(a: Option<char>);
        device_status(a: usize);
        move_forward(a: usize);
        move_backward(a: usize);
        move_down_and_cr(a: usize);
        move_up_and_cr(a: usize);
        put_tab(a: u16);
        backspace();
        carriage_return();
        linefeed();
        bell();
        substitute();
        newline();
        set_horizontal_tabstop();
        scroll_up(a: usize);
        scroll_down(a: usize);
        insert_blank_lines(a: usize);
        delete_lines(a: usize);
        erase_chars(a: usize);
        delete_chars(a: usize);
        move_backward_tabs(a: u16);
        move_forward_tabs(a: u16);
        save_cursor_position();
        restore_cursor_position();
        clear_line(a: LineClearMode);
        clear_screen(a: ClearMode);
        clear_tabs(a: TabulationClearMode);
        set_tabs(a: u16);
        reset_state();
        reverse_index();
        terminal_attribute(a: Attr);
        set_mode(a: Mode);
        unset_mode(a: Mode);
        report_mode(a: Mode);
        set_private_mode(a: PrivateMode);
        unset_private_mode(a: PrivateMode);
        report_private_mode(a: PrivateMode);
        set_scrolling_region(a: usize, b: Option<usize>);
        set_keypad_application_mode();
        unset_keypad_application_mode();
        set_active_charset(a: CharsetIndex);
        configure_charset(a: CharsetIndex, b: StandardCharset);
        set_color(a: usize, b: Rgb);
        dynamic_color_sequence(a: String, b: usize, c: &str);
        reset_color(a: usize);
        clipboard_store(a: u8, b: &[u8]);
        clipboard_load(a: u8, b: &str);
        decaln();
        push_title();
        pop_title();
        text_area_size_pixels();
        text_area_size_chars();
        set_hyperlink(a: Option<Hyperlink>);
        set_mouse_cursor_icon(a: CursorIcon);
        report_keyboard_mode();
        push_keyboard_mode(a: KeyboardModes);
        pop_keyboard_modes(a: u16);
        set_keyboard_mode(a: KeyboardModes, b: KeyboardModesApplyBehavior);
        set_modify_other_keys(a: ModifyOtherKeys);
        report_modify_other_keys();
        set_scp(a: ScpCharPath, b: ScpUpdateMode);
    }
}
