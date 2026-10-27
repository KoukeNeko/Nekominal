//! Pictures in the terminal: Sixel and the kitty graphics protocol, where they go, how they move with their text,
//! what removes them, and the replies programs probe for them with. Cells are 8 x 16 pixels (`CoreConfig` default).

use base64::Engine;
use nekominal_term::{
    alacritty_terminal::grid::Scroll, CoreConfig, CoreEvent, Snapshot, TerminalCore, Unsupported, DA1_WITH_SIXEL,
};

fn core(columns: usize, rows: usize) -> TerminalCore {
    TerminalCore::new(columns, rows, CoreConfig::default())
}

fn snap(core: &mut TerminalCore) -> Snapshot {
    let mut snapshot = Snapshot::default();
    core.snapshot(&mut snapshot);
    snapshot
}

fn replies(core: &mut TerminalCore) -> String {
    core.take_events()
        .iter()
        .filter_map(|e| match e {
            CoreEvent::PtyWrite(bytes) => Some(String::from_utf8_lossy(bytes).into_owned()),
            _ => None,
        })
        .collect()
}

/// Rows and columns of the pictures in view.
fn placed(core: &mut TerminalCore) -> Vec<(i32, i32)> {
    snap(core).images.iter().map(|image| (image.row, image.column)).collect()
}

/// A red Sixel `width` pixels wide and `bands` bands of six tall.
fn sixel(width: usize, bands: usize) -> String {
    let band = format!("!{width}~");
    format!("\x1bPq\"1;1;{width};{}#1;2;100;0;0{}\x1b\\", bands * 6, vec![band; bands].join("-"))
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// A kitty command with an RGBA payload of `width` x `height` opaque white pixels.
fn kitty(keys: &str, width: usize, height: usize) -> String {
    format!("\x1b_G{keys},f=32,s={width},v={height};{}\x1b\\", b64(&vec![255; width * height * 4]))
}

// --- Sixel --------------------------------------------------------------------------------------

#[test]
fn a_sixel_goes_at_the_cursor_and_the_cursor_moves_below_it_to_the_column_it_started_at() {
    let mut core = core(20, 10);
    core.feed(b"\x1b[3;5H");
    core.feed(sixel(20, 6).as_bytes()); // 20 x 36 pixels: 3 columns, 3 rows
    assert_eq!(placed(&mut core), [(2, 4)]);
    assert_eq!(core.cursor_cell(), (5, 4));
    let image = &snap(&mut core).images[0];
    assert_eq!((image.width, image.height, image.cells), (20, 36, None));
    assert_eq!(&image.pixels[..4], &[255, 0, 0, 255]);
}

#[test]
fn decset_8452_leaves_the_cursor_right_of_the_sixel_and_decsdm_draws_at_the_top_left_without_moving() {
    let mut core = core(20, 10);
    core.feed(b"\x1b[?8452h\x1b[2;2H");
    core.feed(sixel(20, 6).as_bytes());
    assert_eq!(core.cursor_cell(), (3, 4), "last row of the picture, the column after it");

    let mut core = self::core(20, 10);
    core.feed(b"\x1b[?80h\x1b[5;5H");
    core.feed(sixel(8, 40).as_bytes()); // 240 pixels: taller than the 160-pixel screen
    assert_eq!(placed(&mut core), [(0, 0)]);
    assert_eq!(core.cursor_cell(), (4, 4), "display mode does not move the cursor");
    assert_eq!(snap(&mut core).images[0].src, [0, 0, 8, 160], "cut at the bottom of the screen");
}

#[test]
fn p2_zero_fills_the_unset_pixels_with_the_background_and_p2_one_leaves_them_clear() {
    let mut core = core(20, 10);
    core.feed(b"\x1bP0;0q\"1;1;2;1#1;2;100;0;0@\x1b\\\x1bP0;1q\"1;1;2;1#1;2;100;0;0@\x1b\\");
    let snapshot = snap(&mut core);
    assert_eq!(snapshot.images[0].pixels[4..8], [12, 12, 12, 255], "Campbell's background");
    assert_eq!(snapshot.images[1].pixels[4..8], [0, 0, 0, 0]);
}

#[test]
fn a_sixel_scrolls_with_its_text_into_the_scrollback_and_can_be_scrolled_back_to() {
    let mut core = core(20, 5);
    core.feed(b"top\r\n");
    core.feed(sixel(8, 3).as_bytes()); // one cell
    for line in 0..10 {
        core.feed(format!("\r\nline {line}").as_bytes());
    }
    assert!(placed(&mut core).is_empty(), "scrolled out of view");
    core.scroll_display(Scroll::Top);
    let snapshot = snap(&mut core);
    assert_eq!(snapshot.row_text(0), "top");
    assert_eq!(snapshot.images.iter().map(|i| (i.row, i.column)).collect::<Vec<_>>(), [(1, 0)]);
}

#[test]
fn the_scrollback_limit_drops_a_sixel_with_its_lines() {
    let mut core = TerminalCore::new(20, 4, CoreConfig { scrollback_lines: 5, ..CoreConfig::default() });
    core.feed(sixel(8, 3).as_bytes());
    for _ in 0..4 {
        core.feed(b"\r\nx");
    }
    core.scroll_display(Scroll::Top);
    assert_eq!(snap(&mut core).images.len(), 1, "still in the scrollback");
    core.scroll_display(Scroll::Bottom);
    for _ in 0..8 {
        core.feed(b"\r\nx");
    }
    core.scroll_display(Scroll::Top);
    assert!(snap(&mut core).images.is_empty(), "trimmed with its line");
}

#[test]
fn text_written_over_a_sixel_erases_it_as_in_xterm_and_text_beside_it_does_not() {
    let mut core = core(20, 10);
    core.feed(b"\x1b[2;3H");
    core.feed(sixel(16, 6).as_bytes()); // rows 1..3, columns 2..4
    core.feed(b"\x1b[1;1Habove\x1b[2;10Hright\x1b[5;1Hbelow");
    assert_eq!(placed(&mut core).len(), 1);
    core.feed(b"\x1b[3;3HX");
    assert!(placed(&mut core).is_empty());

    // Erasing (ECH, as a program clearing a preview does) counts as writing.
    core.feed(b"\x1b[2;3H");
    core.feed(sixel(16, 6).as_bytes());
    core.feed(b"\x1b[2;1H\x1b[5X");
    assert!(placed(&mut core).is_empty());
}

#[test]
fn a_new_sixel_over_an_old_one_replaces_it() {
    let mut core = core(20, 10);
    core.feed(b"\x1b[2;3H");
    core.feed(sixel(16, 6).as_bytes());
    core.feed(b"\x1b[2;3H");
    core.feed(sixel(16, 6).as_bytes());
    assert_eq!(placed(&mut core).len(), 1);
}

#[test]
fn clearing_the_buffer_and_ris_remove_pictures_and_ed2_moves_them_with_the_screen_into_the_scrollback() {
    // ED 2 on the primary screen scrolls it up to its last text: a picture on a blank row below that is blanked.
    let mut core = core(20, 5);
    core.feed(b"text\r\n");
    core.feed(sixel(8, 3).as_bytes());
    core.feed(b"\x1b[H\x1b[2J");
    core.scroll_display(Scroll::Top);
    assert!(placed(&mut core).is_empty());

    let mut core = self::core(20, 5);
    core.feed(b"text\r\n");
    core.feed(sixel(8, 3).as_bytes());
    core.feed(b"prompt\x1b[H\x1b[2J");
    assert!(placed(&mut core).is_empty(), "the screen went into the scrollback");
    core.scroll_display(Scroll::Top);
    assert_eq!(placed(&mut core).len(), 1);
    core.scroll_display(Scroll::Bottom);
    assert!(core.clear_buffer());
    core.scroll_display(Scroll::Top);
    assert!(placed(&mut core).is_empty());

    let mut core = self::core(20, 5);
    core.feed(sixel(8, 3).as_bytes());
    core.feed(kitty("a=T,i=1,q=1", 1, 1).as_bytes());
    core.feed(b"\x1bc");
    assert!(placed(&mut core).is_empty());
    core.feed(b"\x1b_Ga=p,i=1\x1b\\");
    assert_eq!(replies(&mut core), "\x1b_Gi=1;ENOENT:no such image\x1b\\", "RIS frees the pictures too");
}

#[test]
fn a_resize_keeps_pictures_on_their_lines() {
    let mut core = core(20, 6);
    core.feed(b"one\r\ntwo\r\n");
    core.feed(sixel(8, 3).as_bytes());
    core.feed(b"prompt");
    assert_eq!(placed(&mut core), [(2, 0)]);
    core.resize(20, 8);
    assert_eq!(placed(&mut core), [(2, 0)], "more rows");
    core.resize(30, 8);
    assert_eq!(placed(&mut core), [(2, 0)], "wider: the lines are reflowed and the picture put back");
    assert_eq!(snap(&mut core).row_text(2), "");
}

#[test]
fn alternate_screen_pictures_go_when_it_is_left_and_primary_ones_come_back() {
    let mut core = core(20, 6);
    core.feed(sixel(8, 3).as_bytes());
    core.feed(b"\x1b[?1049h");
    assert!(placed(&mut core).is_empty(), "primary pictures are not drawn on the alternate screen");
    core.feed(b"\x1b[3;3H");
    core.feed(kitty("a=T,i=4,q=2", 1, 1).as_bytes());
    assert_eq!(placed(&mut core), [(2, 2)]);
    core.feed(b"\x1b[2J");
    assert!(placed(&mut core).is_empty(), "ED 2 clears the alternate screen");
    core.feed(b"\x1b_Ga=p,i=4,q=2\x1b\\");
    assert_eq!(placed(&mut core).len(), 1);
    core.feed(b"\x1b[?1049l");
    assert_eq!(placed(&mut core), [(0, 0)], "only the primary picture is left");
}

// --- kitty graphics -----------------------------------------------------------------------------

#[test]
fn transmit_and_display_replies_ok_and_moves_the_cursor_right_of_the_picture_on_its_last_row() {
    let mut core = core(40, 10);
    core.feed(b"\x1b[2;3H");
    core.feed(kitty("a=T,i=31", 20, 40).as_bytes()); // 3 x 3 cells
    assert_eq!(replies(&mut core), "\x1b_Gi=31;OK\x1b\\");
    assert_eq!(placed(&mut core), [(1, 2)]);
    assert_eq!(core.cursor_cell(), (3, 5));
}

#[test]
fn quiet_levels_drop_the_ok_and_then_the_errors_and_no_id_means_no_reply() {
    let mut core = core(40, 10);
    core.feed(kitty("a=T,i=1,q=1", 1, 1).as_bytes());
    assert_eq!(replies(&mut core), "");
    core.feed(b"\x1b_Ga=p,i=99,q=1\x1b\\");
    assert_eq!(replies(&mut core), "\x1b_Gi=99;ENOENT:no such image\x1b\\");
    core.feed(b"\x1b_Ga=p,i=99,q=2\x1b\\");
    assert_eq!(replies(&mut core), "");
    core.feed(kitty("a=T", 1, 1).as_bytes());
    assert_eq!(replies(&mut core), "", "an anonymous picture is not answered");
    assert_eq!(placed(&mut core).len(), 2);
}

#[test]
fn an_image_number_gets_an_id_from_the_terminal_and_places_the_newest_picture() {
    let mut core = core(40, 10);
    core.feed(kitty("a=t,I=13", 1, 1).as_bytes());
    let reply = replies(&mut core);
    let id: u32 = reply.strip_prefix("\x1b_Gi=").and_then(|r| r.split(',').next()).and_then(|n| n.parse().ok()).expect(&reply);
    assert_eq!(reply, format!("\x1b_Gi={id},I=13;OK\x1b\\"));
    core.feed(b"\x1b_Ga=p,I=13,p=2\x1b\\");
    assert_eq!(replies(&mut core), format!("\x1b_Gi={id},I=13,p=2;OK\x1b\\"));
    assert_eq!(placed(&mut core).len(), 1);
}

#[test]
fn a_chunked_transfer_is_one_image_and_is_answered_once() {
    let mut core = core(40, 10);
    let data = b64(&[0, 0, 255, 255].repeat(4)); // 2 x 2 blue
    let (first, rest) = data.split_at(8);
    core.feed(format!("\x1b_Ga=T,i=5,f=32,s=2,v=2,m=1;{first}\x1b\\").as_bytes());
    assert_eq!(replies(&mut core), "");
    core.feed(format!("\x1b_Gm=0;{rest}\x1b\\").as_bytes());
    assert_eq!(replies(&mut core), "\x1b_Gi=5;OK\x1b\\");
    let image = &snap(&mut core).images[0];
    assert_eq!((image.width, image.height, &image.pixels[12..16]), (2, 2, &[0, 0, 255, 255][..]));
}

#[test]
fn a_chunked_transfer_past_the_limit_is_refused() {
    let mut core = core(40, 10);
    let chunk = "A".repeat(4 << 20);
    core.feed(b"\x1b_Ga=t,i=8,f=32,s=4096,v=4096,m=1;\x1b\\");
    for _ in 0..(nekominal_term::kitty::MAX_TRANSFER / chunk.len() + 1) {
        core.feed(format!("\x1b_Gm=1;{chunk}\x1b\\").as_bytes());
    }
    assert_eq!(replies(&mut core), "\x1b_Gi=8;EFBIG:the transmission is too large\x1b\\");
}

#[test]
fn placement_keys_crop_scale_offset_and_layer_the_picture() {
    let mut core = core(40, 10);
    core.feed(kitty("a=t,i=2,q=2", 64, 32).as_bytes());
    core.feed(b"\x1b_Ga=p,i=2,x=8,y=4,w=16,h=8,c=4,r=2,X=3,Y=5,z=-1,C=1,q=2\x1b\\");
    let image = &snap(&mut core).images[0];
    assert_eq!((image.src, image.cells, image.offset, image.z), ([8, 4, 16, 8], Some([4, 2]), [3, 5], -1));
    assert_eq!(core.cursor_cell(), (0, 0), "C=1");
    // Only columns given: the rows keep the aspect of the part shown (64 x 32 into 4 columns of 8 pixels: 16 pixels).
    core.feed(b"\x1b_Ga=p,i=2,c=4,C=1,q=2\x1b\\");
    assert_eq!(snap(&mut core).images[1].cells, Some([4, 1]));
    core.feed(b"\x1b_Ga=p,i=2,x=64,q=1\x1b\\");
    assert_eq!(replies(&mut core), "\x1b_Gi=2;EINVAL:the part of the image to show is empty\x1b\\");
}

#[test]
fn a_placement_id_replaces_the_earlier_placement_of_the_same_picture() {
    let mut core = core(40, 10);
    core.feed(kitty("a=t,i=2,q=2", 1, 1).as_bytes());
    core.feed(b"\x1b_Ga=p,i=2,p=1,q=2\x1b\\\x1b[5;5H\x1b_Ga=p,i=2,p=1,q=2\x1b\\");
    assert_eq!(placed(&mut core), [(4, 4)]);
}

#[test]
fn deletes_pick_placements_by_every_key_and_upper_case_frees_the_picture() {
    // Two pictures: i=1 at row 0 column 0 (2 x 1 cells, z 5), i=2 at row 2 column 10 (1 x 1, z 0).
    let setup = || {
        let mut core = core(40, 10);
        core.feed(kitty("a=T,i=1,q=2,z=5,C=1", 16, 16).as_bytes());
        core.feed(b"\x1b[3;11H");
        core.feed(kitty("a=T,i=2,I=7,q=2,C=1", 8, 16).as_bytes());
        core
    };
    let cases: [(&str, usize); 13] = [
        ("d=a", 0),
        ("d=i,i=1", 1),
        ("d=i,i=1,p=9", 2),
        ("d=n,I=7", 1),
        ("d=c", 1),
        ("d=p,x=2,y=1", 1),
        ("d=q,x=1,y=1,z=0", 2),
        ("d=q,x=1,y=1,z=5", 1),
        ("d=x,x=11", 1),
        ("d=y,y=3", 1),
        ("d=z,z=5", 1),
        ("d=r,x=1,y=2", 0),
        ("d=y,y=9", 2),
    ];
    for (keys, left) in cases {
        let mut core = setup();
        core.feed(format!("\x1b[3;11H\x1b_Ga=d,{keys}\x1b\\").as_bytes());
        assert_eq!(placed(&mut core).len(), left, "{keys}");
        assert_eq!(replies(&mut core), "", "{keys}: deletes are not answered");
    }
    let mut core = setup();
    core.feed(b"\x1b_Ga=d,d=i,i=1\x1b\\\x1b_Ga=p,i=1\x1b\\");
    assert_eq!(replies(&mut core), "\x1b_Gi=1;OK\x1b\\", "lower case keeps the picture");
    core.feed(b"\x1b_Ga=d,d=I,i=1\x1b\\\x1b_Ga=p,i=1\x1b\\");
    assert_eq!(replies(&mut core), "\x1b_Gi=1;ENOENT:no such image\x1b\\", "upper case frees it");
}

#[test]
fn a_query_checks_a_transmission_without_keeping_it() {
    let mut core = core(40, 10);
    core.feed(kitty("a=q,i=31", 1, 1).as_bytes());
    assert_eq!(replies(&mut core), "\x1b_Gi=31;OK\x1b\\");
    core.feed(b"\x1b_Ga=q,i=32,f=32,s=1,v=1;AA\x1b\\");
    assert_eq!(replies(&mut core), "\x1b_Gi=32;ENODATA:insufficient image data\x1b\\");
    core.feed(b"\x1b_Ga=p,i=31\x1b\\");
    assert!(replies(&mut core).contains("ENOENT"));
}

#[test]
fn animation_and_unicode_placeholders_are_refused_with_an_error() {
    let mut core = core(40, 10);
    core.feed(b"\x1b_Ga=f,i=1\x1b\\");
    let events = core.take_events();
    assert!(events.iter().any(|e| matches!(e, CoreEvent::Unsupported(Unsupported::KittyAnimation))));
    core.feed(kitty("a=T,i=2,U=1", 1, 1).as_bytes());
    assert_eq!(replies(&mut core), "\x1b_Gi=2;EINVAL:Unicode placeholders are not supported\x1b\\");
}

#[test]
fn a_picture_placed_inside_a_synchronized_update_goes_where_the_update_left_the_cursor() {
    let mut core = core(40, 10);
    core.feed(b"\x1b[?2026h\x1b[4;6H");
    core.feed(kitty("a=T,q=2,C=1", 1, 1).as_bytes());
    core.feed(b"\x1b[?2026l");
    assert_eq!(placed(&mut core), [(3, 5)]);
}

// --- replies ------------------------------------------------------------------------------------

#[test]
fn cell_and_text_area_pixel_sizes_follow_the_cell_size_the_pane_reports() {
    let mut core = core(80, 24);
    core.set_cell_size_px(9, 19);
    core.feed(b"\x1b[16t\x1b[14t\x1b[18t");
    assert_eq!(replies(&mut core), "\x1b[6;19;9t\x1b[4;456;720t\x1b[8;24;80t");
}

#[test]
fn xtsmgraphics_reports_registers_and_geometry_as_xterm_does() {
    let mut core = core(80, 24);
    core.feed(b"\x1b[?1;1;0S\x1b[?1;4;0S\x1b[?2;1;0S\x1b[?2;4;0S\x1b[?3;1;0S\x1b[?2;9;0S");
    assert_eq!(
        replies(&mut core),
        "\x1b[?1;0;1024S\x1b[?1;0;1024S\x1b[?2;0;640;384S\x1b[?2;0;4096;4096S\x1b[?3;1;0S\x1b[?2;2;0S"
    );
}

#[test]
fn the_shells_da1_reply_advertises_sixel() {
    let mut core = TerminalCore::new(20, 5, CoreConfig { da1_reply: Some(DA1_WITH_SIXEL.into()), ..CoreConfig::default() });
    core.feed(b"\x1b[c");
    let reply = replies(&mut core);
    assert!(reply.starts_with("\x1b[?") && reply.trim_end_matches('c').split(';').any(|attribute| attribute == "4"), "{reply:?}");
}

// --- untrusted input ----------------------------------------------------------------------------

#[test]
fn garbage_inside_sixel_and_kitty_strings_never_panics_and_text_after_it_still_prints() {
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let pieces: [&[u8]; 12] = [
        b"\x1bPq",
        b"\x1bP0;1;0q\"",
        b"\x1b_G",
        b"\x1b_Ga=T,f=100;",
        b"\x1b_Ga=d,d=",
        b"\x1b\\",
        b"#1;2;",
        b"!99999",
        b"a=p,i=",
        b"m=1;",
        b"~-$?",
        b"\x1b[?80h",
    ];
    for _ in 0..50 {
        let mut core = core(30, 8);
        for _ in 0..400 {
            let roll = next();
            if roll % 3 == 0 {
                core.feed(pieces[(roll >> 8) as usize % pieces.len()]);
            } else {
                let bytes: Vec<u8> = (0..(roll >> 16) % 16).map(|_| next() as u8).collect();
                core.feed(&bytes);
            }
            core.take_events();
        }
        core.feed(b"\x18\x1b\\\x1bc after");
        let snapshot = snap(&mut core);
        assert!(snapshot.text().contains("after"), "{:?}", snapshot.text());
    }
}
