//! The shell integration in a real interactive PowerShell under ConPTY: what the marks it writes tell the terminal.

use std::{
    collections::BTreeMap,
    thread::sleep,
    time::{Duration, Instant},
};

use nekominal_model::shell_integration::powershell_args;
use nekominal_pane::Pane;
use nekominal_pty::conpty::{SpawnConfig, WidthModel};
use nekominal_term::CoreConfig;

/// Waits up to 30 seconds for `done`, checking the pane's terminal every 100 ms.
fn wait(pane: &Pane, mut done: impl FnMut(&Pane) -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        if done(pane) {
            return true;
        }
        sleep(Duration::from_millis(100));
    }
    false
}

#[test]
#[ignore = "starts Windows PowerShell under ConPTY"]
fn a_typed_command_s_output_is_marked_from_its_first_line_to_its_last_with_its_exit_code() {
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    let config = SpawnConfig {
        program: std::path::Path::new(&root).join("System32\\WindowsPowerShell\\v1.0\\powershell.exe"),
        args: powershell_args(&["-NoLogo".to_owned(), "-NoProfile".to_owned()]),
        cwd: None,
        env: BTreeMap::new(),
        cols: 80,
        rows: 30,
        width_model: WidthModel::Wcswidth,
        inherit_environment: false,
    };
    let pane = Pane::spawn_with(config, CoreConfig::default()).expect("powershell starts under ConPTY");
    let prompted = |pane: &Pane| pane.term.lock().prompt_lines().len();
    assert!(wait(&pane, |pane| prompted(pane) >= 1), "the first prompt is marked");
    pane.send(b"Write-Output one two\r".to_vec());
    let screen = |pane: &Pane| pane.term.lock().text_between((-50, 0), (29, 79));
    assert!(wait(&pane, |pane| prompted(pane) >= 2), "the prompt after the command is marked; the screen:\n{}", screen(&pane));
    pane.send(b"cmd /c exit 3\r".to_vec());
    assert!(wait(&pane, |pane| prompted(pane) >= 3), "and after the failing one");
    let outputs = pane.term.lock().command_outputs();
    let text = pane.term.lock().last_command_output();
    let first = pane.term.lock().text_between((outputs[0].first, 0), (outputs[0].last, 79));
    pane.shutdown();
    assert_eq!(outputs.len(), 1, "the failing command wrote nothing: {outputs:?}");
    assert_eq!(first, "one\ntwo", "the output starts on the line after the command typed");
    assert_eq!(outputs[0].exit, Some(0));
    assert_eq!(text.as_deref(), Some("one\ntwo"));
}
