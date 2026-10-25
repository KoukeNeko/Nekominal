//! A restored pane against a real shell under ConPTY: the text of the last session stays in the scrollback above the
//! separator, and the new shell's prompt comes below it on a clean screen.
//! Run with `cargo test -p nekominal-pane --test restore_session -- --ignored --nocapture`.

use std::{
    collections::BTreeMap,
    sync::Arc,
    thread::sleep,
    time::{Duration, Instant},
};

use nekominal_pane::Pane;
use nekominal_pty::conpty::{SpawnConfig, WidthModel};
use nekominal_term::{CoreConfig, Snapshot};

fn system32(file: &str) -> std::path::PathBuf {
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    std::path::Path::new(&root).join("System32").join(file)
}

const SAVED: &str = "\x1b[0;32mrestored-marker one\x1b[0m\r\nC:\\old> echo restored-marker two\r\nrestored-marker two\r\n";
const LABEL: &str = "Restored 2026-10-25 22:54";

/// Starts `program` restored from `SAVED`, waits until its output contains `prompt`, and checks what the pane holds.
fn restores_above_the_new_prompt(program: &str, args: &[&str], prompt: &str) {
    let config = SpawnConfig {
        program: system32(program),
        args: args.iter().map(|arg| (*arg).to_owned()).collect(),
        cwd: None,
        env: BTreeMap::new(),
        cols: 80,
        rows: 24,
        width_model: WidthModel::Graphemes,
        inherit_environment: true,
    };
    let pane = Pane::spawn_prepared(config, CoreConfig::default(), Arc::new(|| {}), |core| core.show_restored(SAVED, LABEL))
        .expect("the shell starts under ConPTY");
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline && !pane.term.lock().all_text().split(LABEL).nth(1).is_some_and(|after| after.contains(prompt)) {
        sleep(Duration::from_millis(100));
    }
    // Some more time for a late clear or redraw of the shell to show up.
    sleep(Duration::from_millis(1500));
    let (all, screen, history) = {
        let mut core = pane.term.lock();
        let mut snapshot = Snapshot::default();
        core.snapshot(&mut snapshot);
        let screen: Vec<String> = (0..snapshot.rows).map(|row| snapshot.row_text(row)).collect();
        (core.all_text(), screen.join("\n"), core.history_size())
    };
    pane.shutdown();
    println!("--- {program}: {history} lines of scrollback ---\n{all}--- screen ---\n{screen}");

    let at = |needle: &str| all.find(needle).unwrap_or_else(|| panic!("{needle:?} in {all:?}"));
    assert!(at("restored-marker one") < at("restored-marker two"));
    assert!(at("restored-marker two") < at(LABEL), "the old text is above the separator");
    assert!(at(LABEL) < at(prompt), "the new prompt is below it");
    assert_eq!(all.matches("restored-marker").count(), 3, "the old text was shown once and never typed into the shell");
    assert!(history >= 4, "the old text and the separator are in the scrollback");
    assert!(!screen.contains("restored-marker") && !screen.contains(LABEL), "the new shell has the screen to itself: {screen}");
    assert!(screen.contains(prompt), "{screen}");
}

#[test]
#[ignore = "spawns ConPTY"]
fn a_restored_cmd_starts_below_the_text_of_the_last_session() {
    restores_above_the_new_prompt("cmd.exe", &["/k", "prompt", "NEW$G"], "NEW>");
}

#[test]
#[ignore = "spawns ConPTY"]
fn a_restored_powershell_starts_below_the_text_of_the_last_session() {
    restores_above_the_new_prompt(
        "WindowsPowerShell\\v1.0\\powershell.exe",
        &["-NoLogo", "-NoProfile", "-NoExit", "-Command", "function prompt { 'NEWPS> ' }"],
        "NEWPS>",
    );
}
