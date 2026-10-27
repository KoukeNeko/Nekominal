//! Pictures in the pane: Sixel (`DCS q`) and the kitty graphics protocol (`APC G`), placed at the cursor and kept in
//! `images` by the lines they are on, and the replies programs probe for them with (XTSMGRAPHICS, `CSI 16 t`).
//!
//! shortcut: a picture is decoded on the PTY forwarder thread under the terminal lock (the limits keep a PNG at the size
//! cap to about a tenth of a second); a worker thread would have to hold back the cursor move that follows a placement.

use super::*;
use crate::{
    images::{Anchor, ImageStore, MAX_IMAGE_SIDE, Placement},
    kitty::{self, Command, Failure, MAX_TRANSFER, fail},
    sixel::{REGISTERS, SixelImage},
};

/// The most cells a kitty placement may ask to be scaled into, each way: the cursor moves past it a row at a time.
const MAX_PLACEMENT_CELLS: u32 = 1000;

impl TerminalCore {
    fn cell_px(&self) -> (u32, u32) {
        let (width, height) = self.config.cell_size_px;
        (u32::from(width.max(1)), u32::from(height.max(1)))
    }

    fn on_alt(&self) -> bool {
        self.term.mode().contains(TermMode::ALT_SCREEN)
    }

    /// The row and column a picture placed now starts at.
    fn cursor_anchor(&self) -> Option<(Anchor, usize)> {
        let point = self.term.grid().cursor.point;
        let anchor = if self.on_alt() { Anchor::Screen(point.line.0) } else { Anchor::Line(self.line_id(point.line.0)?) };
        Some((anchor, point.column.0))
    }

    /// Inside a synchronized update the emulator has not applied the bytes before the picture yet, so its cursor is
    /// stale: apply them first.
    fn catch_up(&mut self) {
        if self.is_sync_pending() {
            self.stop_sync();
        }
    }

    /// Moves the cursor `down` rows (scrolling at the bottom, as text would) and to `column`.
    fn move_cursor_past(&mut self, down: usize, column: usize) {
        for _ in 0..down {
            self.term.linefeed();
        }
        self.term.goto_col(column.min(self.term.columns() - 1));
    }

    /// The index `LineId`s of grid line 0 (the top of the live screen) have.
    fn screen_top_index(&self) -> i64 {
        self.line_base as i64 + self.term.history_size() as i64
    }

    /// After the emulator ran over some output: a switch to or from the alternate screen leaves it blank (1049 clears
    /// it both ways), so its pictures go; rows `ED 2` blanked lose theirs; text written over a Sixel erases it.
    pub(super) fn images_after_emulator(&mut self, touched: &Touched) {
        let alt = self.on_alt();
        let mut changed = false;
        if alt != self.images_alt {
            self.images_alt = alt;
            changed |= self.images.clear_screen(true);
        }
        // Rows were counted from the lines trimmed before this run; the alternate screen has none.
        let base = if alt { 0 } else { self.line_base as i64 };
        if !touched.cells.is_empty() {
            let cells: Vec<(i64, usize, usize)> = touched.cells.iter().map(|&(row, start, end)| (row + base, start, end)).collect();
            changed |= self.images.erase(&cells, alt, self.line_epoch, false);
        }
        if !touched.cleared.is_empty() {
            let rows: Vec<(i64, usize, usize)> = touched.cleared.iter().map(|&row| (row + base, 0, usize::MAX)).collect();
            changed |= self.images.erase(&rows, alt, self.line_epoch, true);
        }
        if changed {
            self.images.sweep();
            self.dirty = true;
        }
    }

    /// The pictures of the primary screen, each with its row relative to the cursor's, taken out before a reflow.
    /// shortcut: while the alternate screen is shown the primary cursor is not at hand, and those pictures go with the
    /// line ids; keep the primary cursor if that shows.
    pub(super) fn images_before_reflow(&mut self) -> Vec<(Placement, i64)> {
        if self.on_alt() || !self.images.has_placements() {
            return Vec::new();
        }
        let cursor = i64::from(self.term.grid().cursor.point.line.0);
        let taken = self.images.take_primary();
        taken
            .into_iter()
            .filter_map(|placement| {
                let Anchor::Line(id) = placement.anchor else { return None };
                Some((placement, i64::from(self.line_of(id)?) - cursor))
            })
            .collect()
    }

    /// Puts the pictures `images_before_reflow` took back at the same distance from the cursor. Text that re-wrapped
    /// between a picture and the cursor shifts it by as many rows (shortcut: the reflow does not say where each line went).
    pub(super) fn images_after_reflow(&mut self, taken: Vec<(Placement, i64)>) {
        let cursor = i64::from(self.term.grid().cursor.point.line.0);
        for (mut placement, delta) in taken {
            let Ok(line) = i32::try_from(cursor + delta) else { continue };
            let Some(id) = self.line_id(line) else { continue };
            placement.anchor = Anchor::Line(id);
            self.images.place(placement);
        }
        self.images.sweep();
    }

    // --- Sixel ----------------------------------------------------------------------------------

    /// Places a decoded Sixel picture at the cursor (or at the top left in Sixel display mode, DECSET 80) and moves the
    /// cursor as xterm does: to the row below the picture, in the column it started at, or with DECSET 8452 just right
    /// of it on its last row. Display mode leaves the cursor and cuts the picture at the bottom of the screen.
    pub(super) fn place_sixel(&mut self, mut image: SixelImage) {
        self.catch_up();
        if !image.transparent_background {
            let background = self.config.palette.slot(self.term.colors(), 257);
            for pixel in image.pixels.as_chunks_mut::<4>().0 {
                if pixel[3] == 0 {
                    *pixel = [background.r, background.g, background.b, 255];
                }
            }
        }
        let (cw, ch) = self.cell_px();
        let (width, mut height) = (image.width, image.height);
        let (anchor, column) = if self.sixel_display {
            height = height.min(self.term.screen_lines() as u32 * ch);
            let anchor = if self.on_alt() { Some(Anchor::Screen(0)) } else { self.line_id(0).map(Anchor::Line) };
            match anchor {
                Some(anchor) => (anchor, 0),
                None => return,
            }
        } else {
            let Some(at) = self.cursor_anchor() else { return };
            at
        };
        let span = [width.div_ceil(cw), height.div_ceil(ch)];
        let key = self.images.insert(0, 0, image.width, image.height, image.pixels);
        let placement =
            Placement { image: key, id: 0, anchor, column, offset: [0; 2], src: [0, 0, width, height], cells: None, span, z: 0, erasable: true };
        let alt = self.on_alt();
        self.images.drop_covered_sixels(&placement, alt, self.line_epoch);
        self.images.place(placement);
        if !self.sixel_display {
            if self.sixel_cursor_right {
                self.move_cursor_past(span[1] as usize - 1, column + span[0] as usize);
            } else {
                self.move_cursor_past(span[1] as usize, column);
            }
        }
        self.dirty = true;
    }

    /// XTSMGRAPHICS (`CSI ? Pi ; Pa ; Pv S`): the number of colour registers (`Pi` 1) and the largest Sixel picture
    /// (2), as xterm answers. Setting either is accepted and changes nothing: the reply says what holds.
    pub(super) fn graphics_attributes(&mut self, item: u16, action: u16) {
        let (cw, ch) = self.cell_px();
        let screen = |cells: usize, px: u32| (cells as u32).saturating_mul(px).min(MAX_IMAGE_SIDE);
        let reply = match (item, action) {
            (1, 1..=4) => format!("\x1b[?1;0;{REGISTERS}S"),
            (2, 1..=3) => format!("\x1b[?2;0;{};{}S", screen(self.term.columns(), cw), screen(self.term.screen_lines(), ch)),
            (2, 4) => format!("\x1b[?2;0;{MAX_IMAGE_SIDE};{MAX_IMAGE_SIDE}S"),
            // Status 2: the action is not one; 1: the item is not one (ReGIS, 3, is not supported).
            (1 | 2, _) => format!("\x1b[?{item};2;0S"),
            _ => format!("\x1b[?{item};1;0S"),
        };
        self.reply(reply);
    }

    /// `CSI 16 t`: the size of a cell in pixels, `CSI 6 ; height ; width t`.
    pub(super) fn cell_size_report(&mut self) {
        let (cw, ch) = self.cell_px();
        self.reply(format!("\x1b[6;{ch};{cw}t"));
    }

    // --- kitty graphics -------------------------------------------------------------------------

    /// One kitty graphics command (the APC string without its `G`). A transmission in chunks (`m=1`) is collected until
    /// its last chunk, up to `MAX_TRANSFER` bytes of base64.
    pub(super) fn kitty_graphics(&mut self, body: &[u8]) {
        let (control, data) = match body.iter().position(|byte| *byte == b';') {
            Some(at) => (&body[..at], &body[at + 1..]),
            None => (body, &[][..]),
        };
        // A command whose keys do not parse cannot be answered: who sent it is among them.
        let Some(command) = Command::parse(control) else { return };
        let (command, data) = match self.kitty_loading.take() {
            Some((first, mut buffer)) => {
                if buffer.len() + data.len() > MAX_TRANSFER {
                    self.kitty_reply(&first, first.id, Err(Failure { code: "EFBIG", message: "the transmission is too large".into() }));
                    return;
                }
                buffer.extend_from_slice(data);
                if command.more {
                    self.kitty_loading = Some((first, buffer));
                    return;
                }
                // Later chunks may carry only `m` and `q`.
                let quiet = if command.quiet != 0 { command.quiet } else { first.quiet };
                (Command { quiet, ..first }, std::borrow::Cow::Owned(buffer))
            }
            None if command.more && matches!(command.action, b't' | b'T' | b'q') => {
                if data.len() <= MAX_TRANSFER {
                    self.kitty_loading = Some((command, data.to_vec()));
                }
                return;
            }
            None => (command, std::borrow::Cow::Borrowed(data)),
        };
        let mut id = command.id;
        let result = self.kitty_run(&command, &data, &mut id);
        self.kitty_reply(&command, id, result);
    }

    /// Runs a complete command; `id` becomes the image id the reply names (one the terminal chose for `I`).
    fn kitty_run(&mut self, command: &Command, data: &[u8], id: &mut u32) -> Result<(), Failure> {
        match command.action {
            b'q' => kitty::decode(command, data).map(|_| ()),
            b't' | b'T' => {
                if command.action == b'T' && command.placeholder {
                    return self.unsupported_placeholder();
                }
                let decoded = kitty::decode(command, data)?;
                if *id == 0 && command.number != 0 {
                    *id = self.images.free_id();
                }
                let key = self.images.insert(*id, command.number, decoded.width, decoded.height, decoded.rgba);
                if command.action == b'T' {
                    self.kitty_place(command, key)?;
                }
                Ok(())
            }
            b'p' => {
                let image = if command.id != 0 { self.images.by_id(command.id) } else { self.images.by_number(command.number) };
                let Some(image) = image else { return fail("ENOENT", "no such image") };
                let key = image.key;
                *id = image.id;
                self.kitty_place(command, key)
            }
            b'd' => {
                self.kitty_delete(command);
                Ok(())
            }
            b'f' | b'a' | b'c' => {
                self.events.push(CoreEvent::Unsupported(Unsupported::KittyAnimation));
                fail("EINVAL", "animation is not supported")
            }
            _ => fail("EINVAL", "unknown action"),
        }
    }

    fn unsupported_placeholder(&mut self) -> Result<(), Failure> {
        self.events.push(CoreEvent::Unsupported(Unsupported::KittyPlaceholder));
        fail("EINVAL", "Unicode placeholders are not supported")
    }

    /// `ESC _ G i=.. ; OK ESC \` or the error. Nothing for a command that named no image (`i`, `I`) or for a delete, the
    /// OK left out with `q=1` and everything with `q=2`.
    fn kitty_reply(&mut self, command: &Command, id: u32, result: Result<(), Failure>) {
        if (id == 0 && command.number == 0) || command.action == b'd' {
            return;
        }
        let message = match result {
            Ok(()) if command.quiet >= 1 => return,
            Err(_) if command.quiet >= 2 => return,
            Ok(()) => "OK".to_owned(),
            Err(failure) => format!("{}:{}", failure.code, failure.message),
        };
        let mut keys = Vec::new();
        if id != 0 {
            keys.push(format!("i={id}"));
        }
        if command.number != 0 {
            keys.push(format!("I={}", command.number));
        }
        if command.placement != 0 {
            keys.push(format!("p={}", command.placement));
        }
        self.reply(format!("\x1b_G{};{message}\x1b\\", keys.join(",")));
    }

    /// Places picture `key` at the cursor as `command` says: the part `x, y, w, h` of it, scaled into `c` x `r` cells
    /// (one of them alone keeps the aspect), `X, Y` pixels into the first cell, at depth `z`. The cursor then moves
    /// right of the picture on its last row, unless `C=1`.
    fn kitty_place(&mut self, command: &Command, key: u64) -> Result<(), Failure> {
        if command.placeholder {
            return self.unsupported_placeholder();
        }
        self.catch_up();
        let Some(image) = self.images.image(key) else { return fail("ENOENT", "no such image") };
        let (iw, ih) = (image.width, image.height);
        let (x, y) = (command.x.min(iw), command.y.min(ih));
        let w = if command.w == 0 { iw - x } else { command.w.min(iw - x) };
        let h = if command.h == 0 { ih - y } else { command.h.min(ih - y) };
        if w == 0 || h == 0 {
            return fail("EINVAL", "the part of the image to show is empty");
        }
        let (cw, ch) = self.cell_px();
        let (columns, rows) = (command.columns.min(MAX_PLACEMENT_CELLS), command.rows.min(MAX_PLACEMENT_CELLS));
        let scaled = |cells: u32, cell: u32, along: u32, across: u32, other_cell: u32| {
            (u64::from(cells * cell) * u64::from(across)).div_ceil(u64::from(along) * u64::from(other_cell)).clamp(1, u64::from(MAX_PLACEMENT_CELLS)) as u32
        };
        let cells = match (columns, rows) {
            (0, 0) => None,
            (columns, 0) => Some([columns, scaled(columns, cw, w, h, ch)]),
            (0, rows) => Some([scaled(rows, ch, h, w, cw), rows]),
            (columns, rows) => Some([columns, rows]),
        };
        let offset = [command.cell_x.min(cw - 1), command.cell_y.min(ch - 1)];
        let span = cells.unwrap_or([(offset[0] + w).div_ceil(cw), (offset[1] + h).div_ceil(ch)]);
        let Some((anchor, column)) = self.cursor_anchor() else { return fail("EINVAL", "no line at the cursor") };
        self.images.place(Placement {
            image: key,
            id: command.placement,
            anchor,
            column,
            offset,
            src: [x, y, w, h],
            cells,
            span,
            z: command.z,
            erasable: false,
        });
        if !command.no_move {
            self.move_cursor_past(span[1] as usize - 1, column + span[0] as usize);
        }
        self.dirty = true;
        Ok(())
    }

    /// `a=d`: the placements `d` picks go; an upper-case letter also frees the pictures left without one.
    fn kitty_delete(&mut self, command: &Command) {
        let what = if command.delete == 0 { b'a' } else { command.delete };
        let free = what.is_ascii_uppercase();
        let (alt, epoch, top) = (self.on_alt(), self.line_epoch, self.screen_top_index());
        let point = self.term.grid().cursor.point;
        let cursor = (i64::from(point.line.0), point.column.0);
        // A cell (row, column) of the live screen, 0-based, under a placement of the active screen.
        let covers = |p: &Placement, row: Option<i64>, column: Option<usize>| {
            ImageStore::screen_rows(p, alt, epoch, top).is_some_and(|(first, end)| {
                row.is_none_or(|row| (first..end).contains(&row))
                    && column.is_none_or(|column| (p.column..p.column + p.span[0] as usize).contains(&column))
            })
        };
        let (x, y) = (command.x.checked_sub(1).map(|x| x as usize), command.y.checked_sub(1).map(i64::from));
        let placement = command.placement;
        let of_placement = |p: &Placement| placement == 0 || p.id == placement;
        let numbered = self.images.by_number(command.number).map(|image| image.key);
        let changed = match what.to_ascii_lowercase() {
            b'a' => self.images.remove_placements(
                |p, _| ImageStore::screen_rows(p, alt, epoch, top).is_some_and(|(_, end)| end > 0),
                free,
            ),
            b'i' => {
                let id = command.id;
                let gone = self.images.remove_placements(|p, image| image.id == id && of_placement(p), free);
                gone | (free && placement == 0 && self.images.remove_images(|image| image.id == id))
            }
            b'n' => {
                let gone = self.images.remove_placements(|p, image| Some(image.key) == numbered && of_placement(p), free);
                gone | (free && placement == 0 && self.images.remove_images(|image| Some(image.key) == numbered))
            }
            b'r' => {
                let range = command.x..=command.y;
                let gone = self.images.remove_placements(|_, image| range.contains(&image.id), free);
                gone | (free && self.images.remove_images(|image| range.contains(&image.id)))
            }
            b'c' => self.images.remove_placements(|p, _| covers(p, Some(cursor.0), Some(cursor.1)), free),
            b'p' => self.images.remove_placements(|p, _| y.is_some() && x.is_some() && covers(p, y, x), free),
            b'q' => self.images.remove_placements(|p, _| y.is_some() && x.is_some() && covers(p, y, x) && p.z == command.z, free),
            b'x' => self.images.remove_placements(|p, _| x.is_some() && covers(p, None, x), free),
            b'y' => self.images.remove_placements(|p, _| y.is_some() && covers(p, y, None), free),
            b'z' => self.images.remove_placements(|p, _| p.z == command.z, free),
            _ => false,
        };
        self.images.sweep();
        self.dirty |= changed;
    }
}
