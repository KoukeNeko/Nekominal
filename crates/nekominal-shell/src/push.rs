//! The notifications `notify` decided on, pushed on to a phone (`notifications.push`): an ntfy topic, or a webhook of
//! the user's own that takes JSON. One thread sends them with the `curl.exe` Windows ships, as `update.rs` reads the
//! bucket, so the app takes no HTTP library. A push that fails is dropped: a phone that hears about a finished agent a
//! minute late after three retries is worth less than a window that never waits on the network.

use std::sync::OnceLock;
use std::sync::mpsc::{SyncSender, TrySendError, sync_channel};
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use nekominal_model::settings::{Push, PushKind, PushWhen};

use crate::logic::TabState;
use crate::notify::Event;
use crate::util;

/// What one push says.
#[derive(Clone, Debug, PartialEq)]
pub struct Message {
    /// The label of the tab, as the toast shows it.
    pub title: String,
    pub body: String,
    /// What happened, in one word (`kind_of`).
    pub kind: &'static str,
    /// The uid of the tab.
    pub tab: u64,
    /// When, in seconds since 1970.
    pub at: u64,
}

impl Message {
    pub fn new(title: String, body: String, event: &Event, tab: u64) -> Message {
        let at = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
        Message { title, body, kind: kind_of(event), tab, at }
    }
}

/// The word of an event in the webhook's JSON, which the tags and the priority of ntfy are also read from.
pub fn kind_of(event: &Event) -> &'static str {
    match event {
        Event::Agent(TabState::Blocked, _) => "blocked",
        Event::Agent(TabState::Error, _) => "error",
        Event::Agent(..) => "done",
        Event::Bell => "bell",
        Event::Message(_) => "message",
        Event::Command { exit: Some(code), .. } if *code != 0 => "commandFailed",
        Event::Command { .. } => "command",
    }
}

/// Whether a notification goes to the phone now: a target is set, and the user is away from the window unless the
/// setting says always.
pub fn wanted(push: &Push, window_active: bool) -> bool {
    !push.url.is_empty() && (push.when == PushWhen::Always || !window_active)
}

/// Why `url` is not taken, if it is not: the token and the text would cross the network in the clear over anything but
/// HTTPS, so plain HTTP is only for a receiver on this computer.
pub fn check_url(url: &str) -> Result<(), &'static str> {
    if url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("the address has a space or a control character");
    }
    let lower = url.to_ascii_lowercase();
    if lower.starts_with("https://") && host(url).is_some_and(|h| !h.is_empty()) {
        return Ok(());
    }
    if lower.starts_with("http://") && host(url).is_some_and(|h| matches!(h.to_ascii_lowercase().as_str(), "localhost" | "127.0.0.1" | "[::1]")) {
        return Ok(());
    }
    Err("the address is not https:// (http:// only to this computer)")
}

/// The host of `url`, without user, port, path or query: what the log may say of an address.
pub fn host(url: &str) -> Option<&str> {
    let rest = &url[url.find("://")? + 3..];
    let authority = &rest[..rest.find(['/', '?', '#']).unwrap_or(rest.len())];
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    Some(match host.find(']') {
        Some(end) if host.starts_with('[') => &host[..=end],
        _ => host.split(':').next().unwrap_or(host),
    })
}

/// The headers and the body of the POST for the kind of the target.
pub fn request(push: &Push, message: &Message) -> (Vec<(&'static str, String)>, String) {
    let mut headers = Vec::new();
    if !push.token.is_empty() {
        headers.push(("Authorization", format!("Bearer {}", push.token)));
    }
    let body = match push.kind {
        PushKind::Ntfy => {
            headers.push(("Content-Type", "text/plain; charset=utf-8".into()));
            headers.push(("Title", header_text(&message.title)));
            let tags = match message.kind {
                "blocked" => "robot,question",
                "error" | "commandFailed" => "warning",
                "bell" => "bell",
                "message" => "speech_balloon",
                "command" => "white_check_mark",
                _ => "robot",
            };
            headers.push(("Tags", tags.into()));
            if matches!(message.kind, "blocked" | "error" | "commandFailed") {
                headers.push(("Priority", "high".into()));
            }
            message.body.clone()
        }
        PushKind::Webhook => {
            headers.push(("Content-Type", "application/json".into()));
            serde_json::json!({
                "title": message.title,
                "body": message.body,
                "kind": message.kind,
                "tab": message.tab,
                "at": message.at,
            })
            .to_string()
        }
    };
    (headers, body)
}

/// A header value that stays one header: on one line, and in ASCII, with anything else as an RFC 2047 word, which ntfy
/// reads.
fn header_text(text: &str) -> String {
    let line = text.replace(['\r', '\n'], " ");
    if line.is_ascii() {
        line
    } else {
        format!("=?UTF-8?B?{}?=", base64::engine::general_purpose::STANDARD.encode(line))
    }
}

/// The request as a config of curl, read from its standard input: the address, the token and the text never appear on
/// a command line another process can read.
fn curl_config(url: &str, headers: &[(&'static str, String)], body: &str) -> String {
    let quoted = |value: &str| {
        let mut out = String::from("\"");
        for c in value.chars() {
            match c {
                '\\' => out.push_str("\\\\"),
                '"' => out.push_str("\\\""),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                c => out.push(c),
            }
        }
        out.push('"');
        out
    };
    // No --location: a redirect could lead the token to plain HTTP or another host.
    let mut config = format!("silent\nfail\nmax-time = 5\nproto = \"=http,https\"\nurl = {}\n", quoted(url));
    for (name, value) in headers {
        let value = value.replace(['\r', '\n'], " ");
        config.push_str(&format!("header = {}\n", quoted(&format!("{name}: {value}"))));
    }
    // --data-raw, not --data: a body that starts with @ is text, not the name of a file to send.
    config.push_str(&format!("data-raw = {}\n", quoted(body)));
    config
}

/// Sends the request with curl; blocks for up to the five seconds of its time limit and a little more.
fn post(config: &str) -> Result<(), String> {
    use std::io::Write;
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut child = Command::new("curl.exe")
        .args(["--config", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| format!("curl.exe did not start: {e}"))?;
    child.stdin.take().expect("piped").write_all(config.as_bytes()).map_err(|e| format!("curl.exe: {e}"))?;
    let status = child.wait().map_err(|e| format!("curl.exe: {e}"))?;
    // curl's exit code names the failure (6: no host, 22: an HTTP error, 28: the time ran out) without the address.
    if status.success() { Ok(()) } else { Err(format!("curl.exe exited with {}", status.code().unwrap_or(-1))) }
}

/// Pushes are sent one after another; more than this waiting are dropped, so a target that hangs cannot pile them up.
const QUEUE: usize = 8;

/// Hands the push to the thread that sends it; returns at once.
pub fn send(push: &Push, message: Message) {
    static SENDER: OnceLock<SyncSender<(Push, Message)>> = OnceLock::new();
    let sender = SENDER.get_or_init(|| {
        let (sender, jobs) = sync_channel::<(Push, Message)>(QUEUE);
        std::thread::Builder::new()
            .name("push".into())
            .spawn(move || {
                // A failure is logged once until a push gets through again, not once for every agent that finishes.
                let mut failing = false;
                for (push, message) in jobs {
                    let sent = check_url(&push.url).map_err(str::to_owned).and_then(|()| {
                        let (headers, body) = request(&push, &message);
                        post(&curl_config(&push.url, &headers, &body))
                    });
                    match sent {
                        Ok(()) => failing = false,
                        Err(e) if !failing => {
                            failing = true;
                            util::log(&format!("push to {}: {e}", host(&push.url).unwrap_or("?")));
                        }
                        Err(_) => {}
                    }
                }
            })
            .expect("the push thread starts");
        sender
    });
    if let Err(TrySendError::Full(_)) = sender.try_send((push.clone(), message)) {
        util::log("push: the queue is full, a notification was dropped");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(kind: &'static str) -> Message {
        Message { title: "claude · api".into(), body: "Waiting for input\nAllow edit?".into(), kind, tab: 7, at: 1_700_000_000 }
    }

    fn push(kind: PushKind, token: &str) -> Push {
        Push { url: "https://ntfy.sh/cat".into(), kind, token: token.into(), when: PushWhen::Away }
    }

    #[test]
    fn an_event_has_one_word_for_what_happened() {
        assert_eq!(kind_of(&Event::Agent(TabState::Blocked, None)), "blocked");
        assert_eq!(kind_of(&Event::Agent(TabState::Error, None)), "error");
        assert_eq!(kind_of(&Event::Agent(TabState::Done, Some("ok".into()))), "done");
        assert_eq!(kind_of(&Event::Bell), "bell");
        assert_eq!(kind_of(&Event::Message("hi".into())), "message");
        assert_eq!(kind_of(&Event::Command { line: None, exit: Some(1), seconds: 20 }), "commandFailed");
        assert_eq!(kind_of(&Event::Command { line: None, exit: None, seconds: 20 }), "command", "no code said is no failure");
    }

    #[test]
    fn ntfy_gets_the_text_as_the_body_and_the_rest_in_its_headers() {
        let (headers, body) = request(&push(PushKind::Ntfy, ""), &message("blocked"));
        assert_eq!(body, "Waiting for input\nAllow edit?");
        let header = |name| headers.iter().find(|(n, _)| *n == name).map(|(_, v)| v.as_str());
        assert_eq!(header("Tags"), Some("robot,question"));
        assert_eq!(header("Priority"), Some("high"), "an agent that waits needs the user");
        assert_eq!(header("Authorization"), None, "no token, no header");
        assert_eq!(header("Title"), Some("=?UTF-8?B?Y2xhdWRlIMK3IGFwaQ==?="), "a title beyond ASCII is an RFC 2047 word");
        let (headers, _) = request(&push(PushKind::Ntfy, "tk_1"), &Message { title: "build\nnext".into(), ..message("done") });
        let header = |name| headers.iter().find(|(n, _)| *n == name).map(|(_, v)| v.as_str());
        assert_eq!(header("Title"), Some("build next"), "a title is one header");
        assert_eq!(header("Priority"), None, "a finished agent is news, not urgent");
        assert_eq!(header("Tags"), Some("robot"));
        assert_eq!(header("Authorization"), Some("Bearer tk_1"));
    }

    #[test]
    fn a_webhook_gets_json_with_the_title_body_kind_tab_and_time() {
        let (headers, body) = request(&push(PushKind::Webhook, "s3cret"), &message("commandFailed"));
        assert!(headers.contains(&("Content-Type", "application/json".into())));
        assert!(headers.contains(&("Authorization", "Bearer s3cret".into())));
        assert!(!headers.iter().any(|(n, _)| *n == "Title" || *n == "Priority"), "the headers of ntfy are ntfy's");
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "title": "claude · api", "body": "Waiting for input\nAllow edit?", "kind": "commandFailed", "tab": 7, "at": 1_700_000_000u64 })
        );
    }

    #[test]
    fn only_https_is_taken_and_plain_http_only_to_this_computer() {
        assert!(check_url("https://ntfy.sh/mytopic").is_ok());
        assert!(check_url("HTTPS://hooks.example.com:8443/a?b=c").is_ok());
        assert!(check_url("http://localhost:8080/hook").is_ok());
        assert!(check_url("http://127.0.0.1/hook").is_ok());
        assert!(check_url("http://[::1]:9000/").is_ok());
        assert!(check_url("http://ntfy.sh/mytopic").is_err());
        assert!(check_url("http://localhost.evil.com/").is_err());
        assert!(check_url("http://localhost@evil.com/").is_err(), "the host is after the user");
        assert!(check_url("ftp://ntfy.sh/x").is_err());
        assert!(check_url("https://").is_err());
        assert!(check_url("https://ntfy.sh/a b").is_err());
        assert!(check_url("ntfy.sh/mytopic").is_err());
    }

    #[test]
    fn the_log_names_only_the_host_of_an_address() {
        assert_eq!(host("https://user:pw@hooks.example.com:8443/path?token=x#y"), Some("hooks.example.com"));
        assert_eq!(host("https://ntfy.sh?auth=x"), Some("ntfy.sh"));
        assert_eq!(host("http://[::1]:9000/"), Some("[::1]"));
        assert_eq!(host("nothing"), None);
    }

    #[test]
    fn away_pushes_only_while_the_window_is_in_the_background() {
        let mut target = push(PushKind::Ntfy, "");
        assert!(wanted(&target, false));
        assert!(!wanted(&target, true), "at the desk the toast is enough");
        target.when = PushWhen::Always;
        assert!(wanted(&target, true));
        target.url.clear();
        assert!(!wanted(&target, false), "no address is off");
    }

    #[test]
    fn the_curl_config_quotes_what_it_is_handed_and_follows_no_redirect() {
        let config = curl_config("https://ntfy.sh/cat", &[("Title", "a \"b\" \\ c".into())], "line 1\nline 2\t@file");
        assert!(config.contains("url = \"https://ntfy.sh/cat\"\n"));
        assert!(config.contains("header = \"Title: a \\\"b\\\" \\\\ c\"\n"));
        assert!(config.contains("data-raw = \"line 1\\nline 2\\t@file\"\n"));
        assert!(!config.contains("location"));
        assert_eq!(config.lines().count(), 7, "no value breaks a line of the config");
    }

    /// Sends one real push; set NEKOMINAL_PUSH_TEST_URL to an ntfy topic of your own and watch the phone.
    #[test]
    #[ignore = "sends over the network"]
    fn a_real_push_reaches_ntfy() {
        let url = std::env::var("NEKOMINAL_PUSH_TEST_URL").expect("NEKOMINAL_PUSH_TEST_URL");
        let target = Push { url, ..push(PushKind::Ntfy, "") };
        let (headers, body) = request(&target, &message("blocked"));
        post(&curl_config(&target.url, &headers, &body)).unwrap();
    }
}
