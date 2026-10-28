//! A pane handed over (as an update does) and taken by another pane: the new one shows what the old one showed, and the
//! same shell goes on in it, at the same prompt. Both panes are in this process here; `nekominal-pty`'s `handoff` test
//! does it across processes.

use std::{
    collections::BTreeMap,
    sync::Arc,
    thread::sleep,
    time::{Duration, Instant},
};

use nekominal_pane::Pane;
use nekominal_pty::{
    conpty::{SpawnConfig, WidthModel},
    handoff::{Process, duplicate_into},
};
use nekominal_term::CoreConfig;

fn wait_for(pane: &Pane, what: impl Fn(&str) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !what(&pane.term.lock().all_text()) {
        assert!(Instant::now() < deadline, "timed out; the pane holds {:?}", pane.term.lock().all_text());
        sleep(Duration::from_millis(50));
    }
}

#[test]
fn a_pane_taken_over_shows_what_the_old_one_showed_and_the_same_shell_goes_on() {
    let config = SpawnConfig {
        program: std::path::Path::new(&std::env::var_os("SystemRoot").unwrap()).join(r"System32\cmd.exe"),
        args: ["/d", "/k", "prompt", "P$G"].map(String::from).to_vec(),
        cwd: None,
        env: BTreeMap::new(),
        cols: 80,
        rows: 24,
        width_model: WidthModel::Graphemes,
        inherit_environment: true,
    };
    let old = Pane::spawn_with(config, CoreConfig::default()).unwrap();
    wait_for(&old, |text| text.contains("P>"));
    old.send(b"set MARK=kept-in-the-shell\r".to_vec());
    old.send(b"echo before-%MARK%\r".to_vec());
    wait_for(&old, |text| text.contains("before-kept-in-the-shell\n"));

    let parts = old.hand_off().expect("the session is handed over");
    let (screen, cols, rows) = {
        let mut core = old.term.lock();
        (core.handoff_screen(), core.columns() as u16, core.rows() as u16)
    };
    let me = Process::open(std::process::id()).unwrap();
    let taken = duplicate_into(&me, &parts).unwrap();
    // The process that hands over never closes its sessions (it exits); here it simply forgets the pane.
    std::mem::forget(old);

    let new = Pane::adopt_prepared(&taken, cols, rows, CoreConfig::default(), Arc::new(|| {}), |core| core.show_handoff(&screen)).unwrap();
    assert!(new.term.lock().all_text().contains("before-kept-in-the-shell"), "what the old pane showed");
    // The same cmd: the variable it was given before is still set.
    new.send(b"echo after-%MARK%\r".to_vec());
    wait_for(&new, |text| text.contains("after-kept-in-the-shell\n"));
    let text = new.term.lock().all_text();
    assert!(text.find("before-kept-in-the-shell\n").unwrap() < text.find("after-kept-in-the-shell\n").unwrap(), "{text}");
    assert_eq!(text.matches("before-kept-in-the-shell\n").count(), 1, "nothing shown twice: {text}");
    new.shutdown();
}
