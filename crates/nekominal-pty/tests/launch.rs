//! Launches discovered profiles through the public API: the environment contract, the working
//! directory and the Visual Studio developer shells. Profiles that are not installed on the
//! machine are skipped.

use std::{
    path::{Path, PathBuf},
    sync::mpsc::channel,
    time::{Duration, Instant},
};

use nekominal_pty::{
    profiles::{self, Profile},
    session::{Event, Sessions, Sink},
};

const DA1_QUERY: &[u8] = b"\x1b[c";
const DA1_REPLY: &[u8] = b"\x1b[?1;2c";

fn find(select: impl Fn(&Profile) -> bool) -> Option<Profile> {
    let profile = profiles::discover().into_iter().find(select);
    if profile.is_none() {
        eprintln!("skipped: the profile is not installed");
    }
    profile
}

fn scratch_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(name);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Starts `profile` with the default environment plus `extra`, types `input` and returns what the
/// session printed until its shell exited.
fn run(
    profile: &Profile,
    cwd: Option<&Path>,
    extra: &[(&str, &str)],
    input: &str,
    timeout: Duration,
) -> String {
    let mut config = profile.spawn_config(cwd, 160, 40);
    config.env.extend(
        extra
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string())),
    );
    let (tx, rx) = channel();
    let sink: Sink = Box::new(move |event| tx.send(event).is_ok());
    let sessions = Sessions::default();
    let (id, _) = sessions.spawn(&config, sink).expect("spawn");
    let session = sessions.get(id).unwrap();
    session.write(input.as_bytes()).unwrap();

    let deadline = Instant::now() + timeout;
    let mut output = Vec::new();
    loop {
        let Some(left) = deadline.checked_duration_since(Instant::now()) else {
            sessions.close(id);
            panic!(
                "{} did not exit in time; output so far: {:?}",
                profile.id,
                String::from_utf8_lossy(&output)
            );
        };
        match rx.recv_timeout(left) {
            Ok(Event::Data(bytes)) => {
                if bytes
                    .windows(DA1_QUERY.len())
                    .any(|window| window == DA1_QUERY)
                {
                    session.write(DA1_REPLY).unwrap();
                }
                session.ack(bytes.len());
                output.extend(bytes);
            }
            Ok(Event::Exit(_)) => return String::from_utf8_lossy(&output).into_owned(),
            Err(_) => {
                sessions.close(id);
                panic!(
                    "{} did not exit in time; output so far: {:?}",
                    profile.id,
                    String::from_utf8_lossy(&output)
                );
            }
        }
    }
}

/// True when `output` holds `[` followed by two digits and `]`: the expansion of a variable
/// sliced with `:~0,2`, which differs from the command line the console echoes.
fn has_bracketed_number(output: &str) -> bool {
    output
        .as_bytes()
        .windows(4)
        .any(|w| w[0] == b'[' && w[1].is_ascii_digit() && w[2].is_ascii_digit() && w[3] == b']')
}

#[test]
fn the_cmd_profile_gets_the_environment_contract_and_the_requested_directory() {
    let profile = find(|p| p.id == "cmd").expect("cmd is part of Windows");
    let dir = scratch_dir("nekominal-launch-cwd");
    let output = run(
        &profile,
        Some(&dir),
        &[("NEKOMINAL_EXTRA", "overridden")],
        "echo [%TERM_PROGRAM%/%TERM_PROGRAM_VERSION%/%COLORTERM%/%TERM%/%NEKOMINAL_EXTRA%]\r\ncd\r\nexit\r\n",
        Duration::from_secs(30),
    );
    let expected = format!(
        "[Nekominal/{}/truecolor/xterm-256color/overridden]",
        profiles::default_environment()["TERM_PROGRAM_VERSION"]
    );
    assert!(output.contains(&expected), "output was {output:?}");
    assert!(
        output
            .to_lowercase()
            .contains(&dir.to_string_lossy().to_lowercase()),
        "output was {output:?}"
    );
}

#[test]
fn the_job_is_the_shell_alone_at_its_prompt_and_the_program_it_runs_while_that_runs() {
    let profile = find(|p| p.id == "cmd").expect("cmd is part of Windows");
    let (tx, rx) = channel();
    let sink: Sink = Box::new(move |event| tx.send(event).is_ok());
    let sessions = Sessions::default();
    let (id, _) = sessions.spawn(&profile.spawn_config(None, 120, 30), sink).expect("spawn");
    let session = sessions.get(id).unwrap();
    // Drained on the side, as the window does, so that the console never waits on its output.
    let reader = session.clone();
    std::thread::spawn(move || {
        while let Ok(event) = rx.recv() {
            if let Event::Data(bytes) = event {
                if bytes.windows(DA1_QUERY.len()).any(|w| w == DA1_QUERY) {
                    let _ = reader.write(DA1_REPLY);
                }
                reader.ack(bytes.len());
            }
        }
    });
    let wait_for = |want: &dyn Fn(&nekominal_pty::conpty::Job) -> bool| {
        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline {
            if session.job().is_some_and(|job| want(&job)) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        false
    };
    assert!(wait_for(&|job| job.root && job.name.eq_ignore_ascii_case("cmd.exe")), "cmd alone at its prompt");
    session.write(b"ping -n 6 127.0.0.1 >nul\r").unwrap();
    assert!(wait_for(&|job| !job.root && job.name.eq_ignore_ascii_case("PING.EXE")), "the program cmd runs");
    sessions.close(id);
}

#[test]
fn the_agent_environment_reaches_the_child_when_merged_in() {
    let profile = find(|p| p.id == "cmd").expect("cmd is part of Windows");
    let agent: Vec<(String, String)> = profiles::agent_environment().into_iter().collect();
    let extra: Vec<(&str, &str)> = agent
        .iter()
        .map(|(n, v)| (n.as_str(), v.as_str()))
        .collect();
    let output = run(
        &profile,
        None,
        &extra,
        "echo [%FORCE_HYPERLINK%/%CLAUDE_CODE_NATIVE_CURSOR%]\r\nexit\r\n",
        Duration::from_secs(30),
    );
    assert!(output.contains("[1/1]"), "output was {output:?}");
}

#[test]
fn the_developer_command_prompt_sets_up_the_build_environment() {
    let Some(profile) = find(|p| p.id.starts_with("vs-") && p.id.ends_with("-cmd")) else {
        return;
    };
    // `:~0,2` expands only when VsDevCmd defined the variable.
    let output = run(
        &profile,
        None,
        &[],
        "echo [%VSCMD_VER:~0,2%]\r\nexit\r\n",
        Duration::from_secs(60),
    );
    assert!(
        has_bracketed_number(&output),
        "{} output was {output:?}",
        profile.id
    );
}

#[test]
fn the_developer_powershell_sets_up_the_build_environment() {
    let Some(profile) = find(|p| p.id.starts_with("vs-") && p.id.ends_with("-powershell")) else {
        return;
    };
    let output = run(
        &profile,
        None,
        &[],
        "'[' + $env:VSCMD_VER.Substring(0, 2) + ']'\r\nexit\r\n",
        Duration::from_secs(90),
    );
    assert!(
        has_bracketed_number(&output),
        "{} output was {output:?}",
        profile.id
    );
}

#[test]
fn git_bash_starts_in_the_requested_directory() {
    let Some(profile) = find(|p| p.id == "git-bash") else {
        return;
    };
    let dir = scratch_dir("nekominal-launch-bash");
    // `pwd` would print the MSYS mount alias (`/tmp` for the temp directory); `cygpath` maps back.
    let output = run(
        &profile,
        Some(&dir),
        &[],
        "cygpath -w \"$PWD\"\nexit\n",
        Duration::from_secs(60),
    );
    let expected = dir.to_string_lossy().to_lowercase();
    assert!(
        output.to_lowercase().contains(&expected),
        "expected {expected}, output was {output:?}"
    );
}
