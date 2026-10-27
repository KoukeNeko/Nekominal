//! The notifications `notify` decided on, pushed on to a phone (`notifications.push`): an ntfy topic, or a webhook of
//! the user's own that takes JSON. One thread sends them with the `curl.exe` Windows ships, as `update.rs` reads the
//! bucket, so the app takes no HTTP library. A push that fails is dropped: a phone that hears about a finished agent a
//! minute late after three retries is worth less than a window that never waits on the network.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{SyncSender, TrySendError, sync_channel};
use std::sync::{Mutex, OnceLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use base64::Engine;
pub use nekominal_model::settings::{check_push_url as check_url, url_host as host};
use nekominal_model::settings::{Push, PushEvent, PushKind, PushWhen};

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

/// Which of the events the user chose (`notifications.push.events`) a word of `kind_of` belongs to.
pub fn event_of(kind: &str) -> PushEvent {
    match kind {
        "blocked" => PushEvent::Waiting,
        "error" => PushEvent::Failed,
        "command" | "commandFailed" => PushEvent::LongCommand,
        "bell" | "message" => PushEvent::Message,
        _ => PushEvent::Done,
    }
}

/// Whether a notification goes to the phone now: a target is set, the user chose its kind of event, and the user is away
/// from the window unless the setting says always.
pub fn wanted(push: &Push, window_active: bool, event: &Event) -> bool {
    !push.url.is_empty() && push.events.contains(&event_of(kind_of(event))) && (push.when == PushWhen::Always || !window_active)
}

/// What the last test push from the settings page came to.
#[derive(Clone, Debug, PartialEq)]
pub enum Test {
    Sending,
    Sent,
    /// Why it did not get through: curl's exit code or why the address is not taken, in the words of the log.
    Failed(String),
}

/// The last test push and when it was asked for; and whether that changed since the page last looked, so that the page
/// is drawn again only then.
static TEST: Mutex<Option<(Instant, Test)>> = Mutex::new(None);
static TEST_CHANGED: AtomicBool = AtomicBool::new(false);

fn set_test(test: Test, asked: Option<Instant>) {
    if let Ok(mut last) = TEST.lock() {
        let at = asked.or(last.as_ref().map(|(at, _)| *at)).unwrap_or_else(Instant::now);
        *last = Some((at, test));
        TEST_CHANGED.store(true, Ordering::Relaxed);
    }
}

/// What the last test push came to, and when it was sent.
pub fn test_status() -> Option<(Instant, Test)> {
    TEST.lock().ok().and_then(|last| last.clone())
}

/// Whether the result of the test push changed since this was last asked.
pub fn take_test_change() -> bool {
    TEST_CHANGED.swap(false, Ordering::Relaxed)
}

/// Sends a test push through the same thread as the others; its result goes to [`test_status`].
pub fn send_test(push: &Push, title: String, body: String) {
    let at = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    set_test(Test::Sending, Some(Instant::now()));
    enqueue(push, Message { title, body, kind: "message", tab: 0, at }, true);
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
    enqueue(push, message, false);
}

/// Hands a push to the thread that sends it; the result of a `test` goes to [`test_status`].
fn enqueue(push: &Push, message: Message, test: bool) {
    static SENDER: OnceLock<SyncSender<(Push, Message, bool)>> = OnceLock::new();
    let sender = SENDER.get_or_init(|| {
        let (sender, jobs) = sync_channel::<(Push, Message, bool)>(QUEUE);
        std::thread::Builder::new()
            .name("push".into())
            .spawn(move || {
                // A failure is logged once until a push gets through again, not once for every agent that finishes.
                let mut failing = false;
                for (push, message, test) in jobs {
                    let sent = check_url(&push.url).map_err(str::to_owned).and_then(|()| {
                        let (headers, body) = request(&push, &message);
                        post(&curl_config(&push.url, &headers, &body))
                    });
                    if test {
                        set_test(sent.clone().map_or_else(Test::Failed, |()| Test::Sent), None);
                    }
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
    if let Err(TrySendError::Full(_)) = sender.try_send((push.clone(), message, test)) {
        util::log("push: the queue is full, a notification was dropped");
        if test {
            set_test(Test::Failed("the queue is full".into()), None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(kind: &'static str) -> Message {
        Message { title: "claude · api".into(), body: "Waiting for input\nAllow edit?".into(), kind, tab: 7, at: 1_700_000_000 }
    }

    fn push(kind: PushKind, token: &str) -> Push {
        Push { url: "https://ntfy.sh/cat".into(), kind, token: token.into(), when: PushWhen::Away, ..Push::default() }
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
    fn away_pushes_only_while_the_window_is_in_the_background() {
        let waiting = Event::Agent(TabState::Blocked, None);
        let mut target = push(PushKind::Ntfy, "");
        assert!(wanted(&target, false, &waiting));
        assert!(!wanted(&target, true, &waiting), "at the desk the toast is enough");
        target.when = PushWhen::Always;
        assert!(wanted(&target, true, &waiting));
        target.url.clear();
        assert!(!wanted(&target, false, &waiting), "no address is off");
    }

    #[test]
    fn only_the_kinds_of_event_the_user_chose_are_pushed() {
        let target = push(PushKind::Ntfy, "");
        assert!(wanted(&target, false, &Event::Agent(TabState::Error, None)));
        assert!(wanted(&target, false, &Event::Agent(TabState::Done, None)));
        assert!(!wanted(&target, false, &Event::Command { line: None, exit: Some(1), seconds: 20 }), "a long command is not chosen by default");
        assert!(!wanted(&target, false, &Event::Bell));
        let target = Push { events: vec![PushEvent::LongCommand, PushEvent::Message], ..target };
        assert!(wanted(&target, false, &Event::Command { line: None, exit: Some(1), seconds: 20 }));
        assert!(wanted(&target, false, &Event::Message("hi".into())) && wanted(&target, false, &Event::Bell), "a bell is the program's own call");
        assert!(!wanted(&target, false, &Event::Agent(TabState::Blocked, None)));
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

    #[test]
    fn each_kind_of_event_belongs_to_one_choice_and_has_its_tags_and_priority() {
        // (kind, the choice it is pushed under, ntfy's tags, urgent)
        let table = [
            ("blocked", PushEvent::Waiting, "robot,question", true),
            ("error", PushEvent::Failed, "warning", true),
            ("done", PushEvent::Done, "robot", false),
            ("command", PushEvent::LongCommand, "white_check_mark", false),
            ("commandFailed", PushEvent::LongCommand, "warning", true),
            ("bell", PushEvent::Message, "bell", false),
            ("message", PushEvent::Message, "speech_balloon", false),
        ];
        for (kind, event, tags, urgent) in table {
            assert_eq!(event_of(kind), event, "{kind}");
            let (headers, _) = request(&push(PushKind::Ntfy, ""), &message(kind));
            let header = |name| headers.iter().find(|(n, _)| *n == name).map(|(_, v)| v.as_str());
            assert_eq!(header("Tags"), Some(tags), "{kind}");
            assert_eq!(header("Priority").is_some(), urgent, "{kind}");
        }
    }

    #[test]
    fn with_one_choice_on_only_its_events_are_pushed() {
        let events = [
            Event::Agent(TabState::Blocked, None),
            Event::Agent(TabState::Error, None),
            Event::Agent(TabState::Done, None),
            Event::Command { line: None, exit: Some(0), seconds: 30 },
            Event::Command { line: None, exit: Some(2), seconds: 30 },
            Event::Bell,
            Event::Message("hi".into()),
        ];
        for chosen in PushEvent::ALL {
            let target = Push { events: vec![chosen], ..push(PushKind::Webhook, "") };
            for event in &events {
                assert_eq!(wanted(&target, false, event), event_of(kind_of(event)) == chosen, "{chosen:?} {event:?}");
            }
        }
        let none = Push { events: vec![], ..push(PushKind::Webhook, "") };
        assert!(events.iter().all(|event| !wanted(&none, false, event)), "nothing chosen, nothing pushed");
    }

    /// Text that breaks a careless request: quotes, backslashes, line breaks, a tab, emoji, CJK, an escape sequence, and
    /// an @ that curl would read as the name of a file.
    const AWKWARD: &str = "@C:\\x \"quoted\" 'single'\r\nline 2\ttab 😀 構建完成 \u{1b}[31mred\u{1b}[0m";

    #[test]
    fn awkward_text_reaches_the_webhook_as_it_was_and_ntfy_in_one_title_header() {
        let long = AWKWARD.repeat(2000);
        let message = Message { title: AWKWARD.into(), body: long.clone(), ..message("done") };
        let (_, body) = request(&push(PushKind::Webhook, ""), &message);
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!((json["title"].as_str(), json["body"].as_str()), (Some(AWKWARD), Some(long.as_str())));
        let (headers, body) = request(&push(PushKind::Ntfy, ""), &message);
        assert_eq!(body, long, "ntfy takes the body as it is");
        let title = headers.iter().find(|(n, _)| *n == "Title").map(|(_, v)| v.clone()).unwrap();
        let encoded = title.strip_prefix("=?UTF-8?B?").and_then(|t| t.strip_suffix("?=")).unwrap();
        let decoded = String::from_utf8(base64::engine::general_purpose::STANDARD.decode(encoded).unwrap()).unwrap();
        assert_eq!(decoded, AWKWARD.replace(['\r', '\n'], " "), "one line, the rest kept");
        let config = curl_config("https://ntfy.sh/cat", &headers, &body);
        assert_eq!(config.lines().count(), 6 + headers.len(), "no value breaks a line of the config");
    }

    /// A one-shot HTTP server on this computer: answers `status` and hands back the request it got.
    fn receiver(status: &'static str) -> (String, std::thread::JoinHandle<Vec<u8>>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://127.0.0.1:{}/topic", listener.local_addr().unwrap().port());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
            let (mut got, mut buffer) = (Vec::new(), [0u8; 65536]);
            let head_end = loop {
                let n = stream.read(&mut buffer).unwrap();
                got.extend_from_slice(&buffer[..n]);
                if let Some(at) = got.windows(4).position(|w| w == b"\r\n\r\n") {
                    break at + 4;
                }
            };
            let head = String::from_utf8_lossy(&got[..head_end]).to_ascii_lowercase();
            let length: usize = head.lines().find_map(|line| line.strip_prefix("content-length:")).map_or(0, |n| n.trim().parse().unwrap());
            if head.contains("expect: 100-continue") {
                stream.write_all(b"HTTP/1.1 100 Continue\r\n\r\n").unwrap();
            }
            while got.len() < head_end + length {
                let n = stream.read(&mut buffer).unwrap();
                got.extend_from_slice(&buffer[..n]);
            }
            stream.write_all(format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes()).unwrap();
            got
        });
        (url, server)
    }

    #[test]
    fn curl_delivers_the_headers_and_the_body_byte_for_byte() {
        let (url, server) = receiver("200 OK");
        let target = Push { url: url.clone(), ..push(PushKind::Ntfy, "tk_\"secret\"") };
        let message = Message { title: "plain title".into(), body: AWKWARD.repeat(50), ..message("blocked") };
        let (headers, body) = request(&target, &message);
        post(&curl_config(&url, &headers, &body)).unwrap();
        let got = server.join().unwrap();
        let split = got.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
        let head = String::from_utf8_lossy(&got[..split]);
        assert!(head.starts_with("POST /topic "), "{head}");
        for line in ["Authorization: Bearer tk_\"secret\"", "Title: plain title", "Tags: robot,question", "Priority: high", "Content-Type: text/plain; charset=utf-8"] {
            assert!(head.lines().any(|l| l == line), "{line} in {head}");
        }
        assert_eq!(&got[split + 4..], AWKWARD.repeat(50).as_bytes());
    }

    #[test]
    fn a_failed_post_says_curls_exit_code_and_not_the_address() {
        // A port nobody listens on: curl cannot connect (7).
        let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://127.0.0.1:{}/tk_secret", closed.local_addr().unwrap().port());
        drop(closed);
        let error = post(&curl_config(&url, &[("Authorization", "Bearer tk_secret".into())], "x")).unwrap_err();
        assert_eq!(error, "curl.exe exited with 7");
        // An answer of an HTTP error fails too (--fail: 22), instead of counting as sent.
        let (url, server) = receiver("500 Internal Server Error");
        assert_eq!(post(&curl_config(&url, &[], "x")).unwrap_err(), "curl.exe exited with 22");
        server.join().unwrap();
    }

    #[test]
    fn the_test_button_goes_from_sending_to_what_the_push_came_to() {
        let wait_for_result = || {
            let deadline = Instant::now() + std::time::Duration::from_secs(10);
            loop {
                match test_status() {
                    Some((_, Test::Sending)) if Instant::now() < deadline => std::thread::sleep(std::time::Duration::from_millis(10)),
                    Some(status) => return status,
                    None => panic!("no status"),
                }
            }
        };
        // An address that is refused fails without curl, with the reason the settings page shows.
        let refused = Push { url: "http://ntfy.sh/cat".into(), ..push(PushKind::Ntfy, "") };
        send_test(&refused, "t".into(), "b".into());
        let (asked, _) = test_status().unwrap();
        let (at, result) = wait_for_result();
        assert_eq!(result, Test::Failed(check_url(&refused.url).unwrap_err().to_owned()));
        assert_eq!(at, asked, "the time is when it was asked, not when it ended");
        assert!(take_test_change(), "the page is told to draw again");
        assert!(!take_test_change());
        // One that gets through.
        let (url, server) = receiver("200 OK");
        send_test(&Push { url, ..push(PushKind::Webhook, "") }, "t".into(), "b".into());
        assert_eq!(wait_for_result().1, Test::Sent);
        server.join().unwrap();
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
