//! Program status reports from coding agents (`OSC 7501`, Program Status Protocol, revision 0.4).
//! Port of `legacy/tauri-xterm/src/agent-status.ts`.
//!
//! Syntax: `OSC 7501 ; key=value : key=value ST`. A report replaces the whole record for its `id`.
//! Lifetime rules (spec 0.4): `working` and `blocked` are dropped on the next OSC 133 `A` and when
//! the process exits; `done` and `error` persist until dismissed; RIS clears everything, DECSTR does
//! not.

use base64::{
    alphabet,
    engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig},
    Engine,
};

use crate::osc::is_control_or_bidi;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentState {
    Idle,
    Working,
    Done,
    Blocked,
    Error,
}

impl AgentState {
    fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "idle" => Self::Idle,
            "working" => Self::Working,
            "done" => Self::Done,
            "blocked" => Self::Blocked,
            "error" => Self::Error,
            _ => return None,
        })
    }

    fn urgency(self) -> u8 {
        match self {
            Self::Blocked => 4,
            Self::Error => 3,
            Self::Working => 2,
            Self::Done => 1,
            Self::Idle => 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockedKind {
    Permission,
    Question,
    Auth,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentRecord {
    pub id: String,
    pub state: AgentState,
    pub kind: Option<BlockedKind>,
    pub progress: Option<u8>,
    pub app: Option<String>,
    pub title: Option<String>,
    pub message: Option<String>,
}

impl AgentRecord {
    fn new(id: String, state: AgentState) -> Self {
        Self { id, state, kind: None, progress: None, app: None, title: None, message: None }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Report {
    Query,
    Set(AgentRecord),
    Clear { id: String },
}

/// Whole sequence is limited to 4096 bytes; the legacy code applied the limit to the payload.
pub const MAX_SEQUENCE_LENGTH: usize = 4096;
const MAX_ID_DEPTH: usize = 8;
pub const MAX_RECORDS: usize = 256;

/// JS `atob` is "forgiving": padding is optional.
const FORGIVING_BASE64: GeneralPurpose = GeneralPurpose::new(
    &alphabet::STANDARD,
    GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
);

fn decode_base64_text(value: &str) -> Option<String> {
    let bytes = FORGIVING_BASE64.decode(value).ok()?;
    let text = String::from_utf8(bytes).ok()?;
    (!text.chars().any(is_control_or_bidi)).then_some(text)
}

/// Parses the payload after `7501;`. Returns `None` for anything malformed.
pub fn parse_report(data: &str) -> Option<Report> {
    if data.len() > MAX_SEQUENCE_LENGTH {
        return None;
    }
    if data == "?" {
        return Some(Report::Query);
    }

    // A later duplicate key wins, like the Map in the legacy code.
    let mut fields: Vec<(&str, &str)> = Vec::new();
    for pair in data.split(':') {
        let separator = pair.find('=').filter(|&at| at > 0)?;
        let (key, value) = (&pair[..separator], &pair[separator + 1..]);
        match fields.iter_mut().find(|(k, _)| *k == key) {
            Some(slot) => slot.1 = value,
            None => fields.push((key, value)),
        }
    }
    let get = |key: &str| fields.iter().find(|(k, _)| *k == key).map(|(_, v)| *v);

    let id = get("id").unwrap_or("");
    if !id.is_empty()
        && (id.split('/').count() > MAX_ID_DEPTH
            || id.split('/').any(str::is_empty)
            || id.chars().any(is_control_or_bidi))
    {
        return None;
    }

    let state_text = get("state").unwrap_or("");
    if state_text == "clear" {
        return Some(Report::Clear { id: id.to_owned() });
    }
    let state = AgentState::parse(state_text)?;

    let mut record = AgentRecord::new(id.to_owned(), state);
    if state == AgentState::Blocked {
        record.kind = match get("kind") {
            Some("permission") => Some(BlockedKind::Permission),
            Some("question") => Some(BlockedKind::Question),
            Some("auth") => Some(BlockedKind::Auth),
            _ => None,
        };
    }
    if let Some(progress) = get("progress") {
        let is_number = (1..=3).contains(&progress.len()) && progress.bytes().all(|b| b.is_ascii_digit());
        if let Some(value) = is_number.then(|| progress.parse::<u16>().ok()).flatten().filter(|v| *v <= 100) {
            record.progress = Some(value as u8);
        }
    }
    if let Some(app) = get("app").filter(|a| !a.is_empty() && !a.chars().any(is_control_or_bidi)) {
        record.app = Some(app.to_owned());
    }
    if let Some(title) = get("title").filter(|t| !t.is_empty()) {
        record.title = decode_base64_text(title);
    }
    if let Some(message) = get("msg").filter(|m| !m.is_empty()) {
        record.message = decode_base64_text(message);
    }
    Some(Report::Set(record))
}

/// Records for one terminal pane. Insertion order is kept so that equally urgent records resolve
/// to the oldest one, like the `Map` in the legacy implementation.
#[derive(Default, Debug)]
pub struct AgentStatus {
    records: Vec<AgentRecord>,
}

impl AgentStatus {
    pub fn apply(&mut self, report: &Report) -> bool {
        match report {
            Report::Query => false,
            Report::Clear { id } => {
                let before = self.records.len();
                self.records.retain(|r| r.id != *id);
                self.records.len() != before
            }
            Report::Set(record) => {
                match self.records.iter().position(|r| r.id == record.id) {
                    Some(at) => self.records[at] = record.clone(),
                    None if self.records.len() >= MAX_RECORDS => return false,
                    None => self.records.push(record.clone()),
                }
                true
            }
        }
    }

    /// `working` and `blocked` do not survive the next prompt or the program's exit.
    pub fn drop_active(&mut self) -> bool {
        let before = self.records.len();
        self.records.retain(|r| !matches!(r.state, AgentState::Working | AgentState::Blocked));
        self.records.len() != before
    }

    /// Full reset (RIS).
    pub fn clear(&mut self) -> bool {
        let had = !self.records.is_empty();
        self.records.clear();
        had
    }

    /// The most urgent record, which is what a tab or sidebar row should show.
    pub fn summary(&self) -> Option<&AgentRecord> {
        let mut best: Option<&AgentRecord> = None;
        for record in &self.records {
            if best.is_none_or(|b| record.state.urgency() > b.state.urgency()) {
                best = Some(record);
            }
        }
        best.filter(|r| r.state != AgentState::Idle)
    }

    /// Acknowledges a finished or failed report once the user has seen it.
    pub fn dismiss_finished(&mut self) -> bool {
        let before = self.records.len();
        self.records.retain(|r| !matches!(r.state, AgentState::Done | AgentState::Error));
        self.records.len() != before
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b64(text: &str) -> String {
        base64::engine::general_purpose::STANDARD.encode(text)
    }

    fn set(data: &str) -> Report {
        parse_report(data).expect("valid report")
    }

    fn record(data: &str) -> AgentRecord {
        match set(data) {
            Report::Set(r) => r,
            other => panic!("not a set report: {other:?}"),
        }
    }

    #[test]
    fn query_and_clear() {
        assert_eq!(parse_report("?"), Some(Report::Query));
        assert_eq!(parse_report("state=clear:id=x"), Some(Report::Clear { id: "x".into() }));
    }

    #[test]
    fn working_with_progress() {
        let r = record("state=working:progress=40:app=claude");
        assert_eq!(r.state, AgentState::Working);
        assert_eq!(r.progress, Some(40));
        assert_eq!(r.app.as_deref(), Some("claude"));
        assert_eq!(r.id, "");
    }

    #[test]
    fn decodes_base64_utf8_title_and_message() {
        let r = record(&format!(
            "state=blocked:kind=permission:id=a/b:title={}:msg={}",
            b64("編輯檔案"),
            b64("允許嗎")
        ));
        assert_eq!(r.kind, Some(BlockedKind::Permission));
        assert_eq!(r.title.as_deref(), Some("編輯檔案"));
        assert_eq!(r.message.as_deref(), Some("允許嗎"));
        // unpadded input is accepted like JS atob
        let unpadded = b64("ab").trim_end_matches('=').to_owned();
        assert_eq!(record(&format!("state=done:msg={unpadded}")).message.as_deref(), Some("ab"));
    }

    #[test]
    fn drops_text_with_control_or_bidi_characters() {
        let r = record(&format!("state=done:msg={}", b64("hi\u{202e}evil")));
        assert_eq!(r.message, None);
        assert_eq!(r.state, AgentState::Done);
    }

    #[test]
    fn kind_only_applies_to_blocked() {
        assert_eq!(record("state=working:kind=permission").kind, None);
        assert_eq!(record("state=blocked:kind=nonsense").kind, None);
    }

    #[test]
    fn rejects_malformed_input() {
        assert_eq!(parse_report("state=sleeping"), None);
        assert_eq!(parse_report("nonsense"), None);
        assert_eq!(parse_report("state=working:id=a//b"), None);
        assert_eq!(parse_report(&format!("state=working:id={}", ["x"; 9].join("/"))), None);
        assert_eq!(parse_report(&format!("state=working:app={}", "x".repeat(5000))), None);
        assert_eq!(parse_report("=x"), None);
        assert_eq!(parse_report("id=a"), None); // no state
    }

    #[test]
    fn out_of_range_progress_is_ignored() {
        assert_eq!(record("state=working:progress=101").progress, None);
        assert_eq!(record("state=working:progress=1000").progress, None);
        assert_eq!(record("state=working:progress=-1").progress, None);
        assert_eq!(record("state=working:progress=100").progress, Some(100));
    }

    #[test]
    fn summary_shows_the_most_urgent_record() {
        let mut status = AgentStatus::default();
        for data in ["state=working:id=a", "state=blocked:id=b", "state=done:id=c"] {
            status.apply(&set(data));
        }
        assert_eq!(status.summary().map(|r| r.id.as_str()), Some("b"));
    }

    #[test]
    fn a_report_replaces_the_whole_record() {
        let mut status = AgentStatus::default();
        status.apply(&set("state=working:progress=10"));
        status.apply(&set("state=working"));
        assert_eq!(status.summary(), Some(&AgentRecord::new(String::new(), AgentState::Working)));
    }

    #[test]
    fn idle_is_not_shown() {
        let mut status = AgentStatus::default();
        status.apply(&set("state=idle"));
        assert_eq!(status.summary(), None);
    }

    #[test]
    fn working_and_blocked_are_dropped_at_the_next_prompt_done_and_error_stay() {
        let mut status = AgentStatus::default();
        for data in ["state=working:id=a", "state=blocked:id=b", "state=done:id=c"] {
            status.apply(&set(data));
        }
        assert!(status.drop_active());
        assert_eq!(status.summary().map(|r| r.id.as_str()), Some("c"));
        assert!(!status.drop_active());
    }

    #[test]
    fn finished_records_can_be_dismissed() {
        let mut status = AgentStatus::default();
        status.apply(&set("state=error"));
        assert!(status.dismiss_finished());
        assert_eq!(status.summary(), None);
    }

    #[test]
    fn clear_and_reset_remove_records_and_cap_is_enforced() {
        let mut status = AgentStatus::default();
        status.apply(&set("state=working:id=a"));
        assert!(status.apply(&set("state=clear:id=a")));
        assert_eq!(status.summary(), None);
        status.apply(&set("state=done:id=b"));
        assert!(status.clear());

        for i in 0..MAX_RECORDS {
            assert!(status.apply(&set(&format!("state=working:id=r{i}"))));
        }
        assert!(!status.apply(&set("state=working:id=overflow")));
        assert!(status.apply(&set("state=done:id=r0"))); // replacing an existing id is fine
        assert_eq!(status.len(), MAX_RECORDS);
    }
}
