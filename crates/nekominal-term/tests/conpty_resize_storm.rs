//! ConPTY experiments, part 7: resizing while the child floods output.
//! Run with: `cargo test --test conpty_resize_storm -- --ignored --nocapture --test-threads=1`
//!
//! The front end resizes the emulator first and the PTY right after (under one lock). ConPTY answers with a
//! repaint of its own buffer at the new size, interleaved with whatever the child keeps writing. Whatever it
//! sends, the emulator must keep a consistent screen: cursor inside the grid, damage inside the grid, the
//! scrollback within its limit, no emulator panic, and the output that follows still lands on screen.

mod common;

use std::time::Duration;

use common::*;
use nekominal_term::{Damage, Snapshot};

struct Rng(u64);

impl Rng {
    fn below(&mut self, n: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % n
    }
}

#[test]
#[ignore = "spawns ConPTY"]
fn resize_during_a_flood_keeps_cursor_damage_and_scrollback_consistent() {
    let args = vec!["-NoLogo".into(), "-NoProfile".into(), "-File".into(), script("burst.ps1"), "60000".into()];
    let mut host = Host::spawn(pwsh(), &args, HostOptions { cols: 100, rows: 30, ..HostOptions::default() });
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let mut snapshot = Snapshot::default();
    let mut resizes = 0;
    let mut partial_frames = 0;

    while host.exit.is_none() && host.started.elapsed() < Duration::from_secs(90) {
        host.pump(Duration::from_millis(15), |_| false);
        let (columns, rows) = (40 + rng.below(100) as u16, 8 + rng.below(40) as u16);
        // Emulator first, then the PTY, like the front end does.
        host.core.resize(columns as usize, rows as usize);
        host.session.resize(columns, rows).expect("resize the PTY");
        resizes += 1;

        host.core.snapshot(&mut snapshot);
        assert_eq!(snapshot.damage, Damage::Full, "resize {resizes}: a resize damages everything");
        host.pump(Duration::from_millis(15), |_| false);
        host.core.snapshot(&mut snapshot);
        let (c, r) = (snapshot.columns, snapshot.rows);
        assert_eq!(snapshot.cells.len(), c * r, "resize {resizes}");
        let (row, column) = snapshot.cursor.position.expect("the view is at the bottom");
        assert!(row < r && column < c, "resize {resizes}: cursor ({row},{column}) outside {c}x{r}");
        if let Damage::Lines(lines) = &snapshot.damage {
            partial_frames += 1;
            for l in lines {
                assert!(l.row < r && l.left <= l.right && l.right < c, "resize {resizes}: {l:?} outside {c}x{r}");
            }
        }
        assert!(host.core.history_size() <= host.core.scrollback_limit(), "resize {resizes}");
    }

    // Let the child finish at a stable size.
    host.core.resize(100, 30);
    host.session.resize(100, 30).expect("resize the PTY");
    host.pump_until_exit(Duration::from_secs(60));
    host.pump(Duration::from_millis(300), |_| false);
    println!(
        "{resizes} resizes in {:.1?}, {partial_frames} partial-damage frames checked, history {} lines, exit {:?}",
        host.started.elapsed(),
        host.core.history_size(),
        host.exit
    );
    assert!(resizes > 20, "the storm must actually resize, did {resizes}");
    assert!(!host.events.iter().any(|e| e.starts_with("EmulatorPanic")), "{:?}", host.events);
    assert!(host.text().contains("BURST-DONE"), "output after the storm still reaches the screen:\n{}", host.text());
}
