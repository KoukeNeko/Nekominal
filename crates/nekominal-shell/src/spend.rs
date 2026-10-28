//! What the agents cost today, for the bottom of the agents panel.
//!
//! Claude Code hands its status line the cost of the whole session so far (`cost.total_cost_usd`), and the status line
//! command (`agent::statusline_main`) notes it in `%LOCALAPPDATA%\Nekominal\spend.json` under the local date and the id of
//! the session: the highest seen that day. What a day cost is what each of its sessions grew by that day: its highest
//! then, less the highest of the last earlier day it was seen (a session resumed from yesterday counts only today's
//! part). Every window of every process reads the same file; the commands that write it take `spend.lock` first, so the
//! writes of two sessions that end together do not undo one another.

use std::{collections::BTreeMap, path::PathBuf};

/// Day (`YYYY-MM-DD`, which sorts as dates do) to session id to the highest cost seen that day, in US dollars.
type Ledger = BTreeMap<String, BTreeMap<String, f64>>;

/// The days kept: enough for any session resumed after a while, little enough for a file read on every status line.
const KEPT_DAYS: usize = 31;

fn ledger_file() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("Nekominal").join("spend.json"))
}

/// The date of this machine's clock, as the ledger keys it.
pub fn today() -> String {
    // SAFETY: fills and returns a plain struct.
    let now = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    format!("{:04}-{:02}-{:02}", now.wYear, now.wMonth, now.wDay)
}

/// Notes that `session` had cost `cost` by `day`; true when the ledger changed. A cost below what the day has is a late
/// write of an older total and changes nothing.
fn note(ledger: &mut Ledger, day: &str, session: &str, cost: f64) -> bool {
    if !cost.is_finite() || cost < 0.0 || session.is_empty() {
        return false;
    }
    let seen = ledger.entry(day.to_owned()).or_default().entry(session.to_owned()).or_insert(-1.0);
    if cost <= *seen {
        return false;
    }
    *seen = cost;
    while ledger.len() > KEPT_DAYS {
        ledger.pop_first();
    }
    true
}

/// What the sessions cost on `day`: what each grew by since the last earlier day it was seen.
fn spent(ledger: &Ledger, day: &str) -> f64 {
    let Some(sessions) = ledger.get(day) else { return 0.0 };
    sessions
        .iter()
        .map(|(session, cost)| {
            let before = ledger.range(..day.to_owned()).rev().find_map(|(_, earlier)| earlier.get(session)).copied().unwrap_or(0.0);
            (cost - before).max(0.0)
        })
        .sum()
}

fn read() -> Ledger {
    ledger_file().and_then(|file| std::fs::read_to_string(file).ok()).and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
}

/// Notes the cost of the session in what Claude Code handed its status line.
pub fn record(status: &serde_json::Value) {
    let session = status.get("session_id").and_then(serde_json::Value::as_str).unwrap_or_default();
    let Some(cost) = status.pointer("/cost/total_cost_usd").and_then(serde_json::Value::as_f64) else { return };
    // One writer at a time: two agents that finish together would each write the ledger they read, and the session of
    // the one written over, if it ended, would never be told again. Held until the write is done.
    let _lock = ledger_file().and_then(|file| lock(&file.with_extension("lock")));
    let mut ledger = read();
    if note(&mut ledger, &today(), session, cost)
        && let Some(file) = ledger_file()
        && let Ok(text) = serde_json::to_string(&ledger)
    {
        crate::agent::write_whole(&file, &text);
    }
}

/// `file` opened for this process alone (no sharing), waited for a little while another holds it; None when it stays held,
/// and the write goes on without it rather than not at all.
fn lock(file: &std::path::Path) -> Option<std::fs::File> {
    use std::os::windows::fs::OpenOptionsExt;
    for _ in 0..50 {
        match std::fs::OpenOptions::new().write(true).create(true).truncate(false).share_mode(0).open(file) {
            Ok(held) => return Some(held),
            Err(_) => std::thread::sleep(std::time::Duration::from_millis(10)),
        }
    }
    None
}

/// What the sessions cost today, as far as the ledger knows.
pub fn today_total() -> f64 {
    spent(&read(), &today())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lock_is_one_holders_at_a_time_and_free_again_once_dropped() {
        let file = std::env::temp_dir().join(format!("nekominal-spend-{}.lock", std::process::id()));
        let held = lock(&file).expect("free");
        let started = std::time::Instant::now();
        assert!(lock(&file).is_none(), "held by the first");
        assert!(started.elapsed() >= std::time::Duration::from_millis(400), "it waited before giving up");
        drop(held);
        assert!(lock(&file).is_some(), "free again");
        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn a_day_costs_what_its_sessions_grew_by_that_day() {
        let mut ledger = Ledger::new();
        assert!(note(&mut ledger, "2026-10-24", "a", 1.00));
        assert!(note(&mut ledger, "2026-10-25", "a", 1.50), "resumed the next day");
        assert!(note(&mut ledger, "2026-10-25", "b", 0.30));
        assert!(note(&mut ledger, "2026-10-25", "b", 0.80), "a session's total only grows");
        assert!(!note(&mut ledger, "2026-10-25", "b", 0.40), "a late write of an older total changes nothing");
        assert!((spent(&ledger, "2026-10-25") - 1.30).abs() < 1e-9, "0.50 of a and 0.80 of b");
        assert!((spent(&ledger, "2026-10-24") - 1.00).abs() < 1e-9);
        assert_eq!(spent(&ledger, "2026-10-26"), 0.0, "nothing yet today");
    }

    #[test]
    fn the_ledger_keeps_a_month_and_refuses_what_is_not_a_cost() {
        let mut ledger = Ledger::new();
        for day in 1..=40 {
            note(&mut ledger, &format!("2026-01-{day:02}"), "s", f64::from(day));
        }
        assert_eq!(ledger.len(), KEPT_DAYS);
        assert_eq!(ledger.keys().next().map(String::as_str), Some("2026-01-10"), "the oldest days go");
        assert!(!note(&mut ledger, "2026-02-01", "s", f64::NAN));
        assert!(!note(&mut ledger, "2026-02-01", "s", -1.0));
        assert!(!note(&mut ledger, "2026-02-01", "", 1.0), "a status without a session id is not counted");
        assert!(note(&mut ledger, "2026-02-01", "t", 0.0), "a session that has cost nothing yet is noted");
    }

    #[test]
    fn the_ledger_reads_back_from_its_json() {
        let mut ledger = Ledger::new();
        note(&mut ledger, "2026-10-25", "a", 0.25);
        let text = serde_json::to_string(&ledger).unwrap();
        assert_eq!(text, r#"{"2026-10-25":{"a":0.25}}"#);
        assert_eq!(serde_json::from_str::<Ledger>(&text).unwrap(), ledger);
    }

    #[test]
    fn today_is_a_date_the_ledger_sorts_by() {
        let day = today();
        assert_eq!(day.len(), 10);
        assert!(day.chars().enumerate().all(|(i, c)| if i == 4 || i == 7 { c == '-' } else { c.is_ascii_digit() }), "{day}");
    }
}
