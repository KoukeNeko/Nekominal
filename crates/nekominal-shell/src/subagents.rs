//! The subagents of a Claude Code session, as Claude Code itself keeps them next to the session's transcript
//! (`<session>.jsonl` and the folder `<session>/`). No hook is needed: every subagent gets `subagents/agent-<id>.meta.json`
//! when it starts (its type and title, and `"finished": true` once it ends), and every workflow appends to
//! `subagents/workflows/<run>/journal.jsonl` as its agents start and end, writes its script to `workflows/scripts/` when it
//! is launched and its outcome to `workflows/<run>.json` when it ends. The window reads them again only when the folder
//! changed or something in it still runs.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::SystemTime,
};

/// How a subagent, or an agent of a workflow, stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Running,
    Done,
    Failed,
}

/// A subagent the session started with the Agent tool.
#[derive(Clone, Debug, PartialEq)]
pub struct Agent {
    pub id: String,
    /// Its kind (`Explore`, `general-purpose`, a plugin's `name:agent`).
    pub kind: String,
    /// The title the session gave it (the Agent tool's `description`).
    pub title: String,
    pub started: SystemTime,
    /// When it ended, as far as the files tell: when its meta file was written for the last time.
    pub finished: Option<SystemTime>,
    /// The last thing it said, once it ended.
    pub reply: Option<String>,
}

/// An agent of a workflow, as its journal tells it.
#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    pub id: String,
    pub label: String,
    pub state: State,
    /// The first line of what it handed back.
    pub result: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Phase {
    pub title: String,
    /// In the order they started.
    pub steps: Vec<Step>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Workflow {
    pub run: String,
    pub name: String,
    pub summary: Option<String>,
    pub started: SystemTime,
    /// When it ended: its outcome was written.
    pub finished: Option<SystemTime>,
    pub phases: Vec<Phase>,
}

impl Workflow {
    fn count(&self, state: State) -> usize {
        self.phases.iter().flat_map(|phase| &phase.steps).filter(|step| step.state == state).count()
    }
}

/// The subagents of the session that are running, with those that ran alongside them: what the bar and its card show.
/// Empty when nothing runs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Subagents {
    /// Running first, then by when they started.
    pub agents: Vec<Agent>,
    pub workflows: Vec<Workflow>,
}

impl Subagents {
    pub fn running(&self) -> usize {
        self.agents.iter().filter(|a| a.finished.is_none()).count() + self.workflows.iter().map(|w| w.count(State::Running)).sum::<usize>()
    }

    pub fn done(&self) -> usize {
        self.agents.iter().filter(|a| a.finished.is_some()).count() + self.workflows.iter().map(|w| w.count(State::Done) + w.count(State::Failed)).sum::<usize>()
    }
}

/// The folder Claude Code keeps a session's subagents in, beside its transcript.
pub fn folder(transcript: &Path) -> PathBuf {
    transcript.with_extension("")
}

/// When the folder of the subagents was last changed: a new subagent or workflow adds files to it.
pub fn stamp(folder: &Path) -> Option<SystemTime> {
    std::fs::metadata(folder.join("subagents")).ok()?.modified().ok()
}

/// Reads the subagents of the session whose folder is `folder`. `old` is the last read, whose replies are kept instead of
/// read again from the transcripts.
pub fn read(folder: &Path, old: &Subagents) -> Subagents {
    let replies: HashMap<&str, &String> = old.agents.iter().filter_map(|a| Some((a.id.as_str(), a.reply.as_ref()?))).collect();
    let dir = folder.join("subagents");
    let mut agents = Vec::new();
    for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(id) = name.strip_prefix("agent-").and_then(|n| n.strip_suffix(".meta.json")) else { continue };
        let Ok(meta) = entry.metadata() else { continue };
        let Some(value) = std::fs::read_to_string(entry.path()).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()) else { continue };
        let Some(mut agent) = parse_meta(id, &value, meta.created().unwrap_or(SystemTime::UNIX_EPOCH), meta.modified().ok()) else { continue };
        if agent.finished.is_some() {
            agent.reply = replies.get(id).map(|r| (*r).clone()).or_else(|| last_reply(&dir.join(format!("agent-{id}.jsonl"))));
        }
        agents.push(agent);
    }
    let mut workflows = Vec::new();
    for entry in std::fs::read_dir(dir.join("workflows")).into_iter().flatten().flatten() {
        let run = entry.file_name().to_string_lossy().into_owned();
        let journal = entry.path().join("journal.jsonl");
        let Ok(text) = std::fs::read_to_string(&journal) else { continue };
        let started = std::fs::metadata(&journal).and_then(|m| m.created()).unwrap_or(SystemTime::UNIX_EPOCH);
        let outcome_file = folder.join("workflows").join(format!("{run}.json"));
        let outcome: Option<serde_json::Value> = std::fs::read_to_string(&outcome_file).ok().and_then(|t| serde_json::from_str(&t).ok());
        let finished = outcome.as_ref().and_then(|_| std::fs::metadata(&outcome_file).ok()?.modified().ok());
        let script = script_of(&folder.join("workflows").join("scripts"), &run);
        let name = outcome.as_ref().and_then(|o| o.get("workflowName")?.as_str().map(str::to_owned)).or_else(|| script.as_ref().map(|(name, _)| name.clone())).unwrap_or_else(|| run.clone());
        let summary = outcome.as_ref().and_then(|o| o.get("summary")?.as_str().map(str::to_owned)).or_else(|| script.and_then(|(_, text)| script_description(&text)));
        workflows.push(Workflow { phases: parse_journal(&text, outcome.as_ref()), run, name, summary, started, finished });
    }
    batch(agents, workflows)
}

/// A subagent from its meta file; `None` for an agent of a workflow, which its journal tells.
fn parse_meta(id: &str, meta: &serde_json::Value, created: SystemTime, modified: Option<SystemTime>) -> Option<Agent> {
    let kind = meta.get("agentType").and_then(|v| v.as_str()).unwrap_or_default();
    if kind == "workflow-subagent" || meta.get("workflowPhase").is_some() {
        return None;
    }
    let finished = meta.get("finished").and_then(serde_json::Value::as_bool) == Some(true);
    Some(Agent {
        id: id.to_owned(),
        kind: kind.to_owned(),
        title: meta.get("description").and_then(|v| v.as_str()).unwrap_or_default().to_owned(),
        started: created,
        finished: if finished { Some(modified.unwrap_or(created)) } else { None },
        reply: None,
    })
}

/// The phases of a workflow from its journal: an agent runs from `started` until its `result`. `outcome`, written when the
/// workflow ended, has the state of every agent, failures among them.
// shortcut: a failed agent is read from the outcome's `state`, or a journal line of another type than started/result
// naming it; Claude Code's journal of a failure was never seen. Check it against a real failing workflow.
fn parse_journal(text: &str, outcome: Option<&serde_json::Value>) -> Vec<Phase> {
    let mut phases: Vec<Phase> = Vec::new();
    let mut at: HashMap<String, (usize, usize)> = HashMap::new();
    for line in text.lines() {
        let Ok(event) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        let field = |key: &str| event.get(key).and_then(|v| v.as_str()).map(str::to_owned);
        let (Some(kind), Some(id)) = (field("type"), field("agentId")) else { continue };
        match kind.as_str() {
            "started" => {
                let title = field("phase").unwrap_or_default();
                let p = phases.iter().position(|p| p.title == title).unwrap_or_else(|| {
                    phases.push(Phase { title, steps: Vec::new() });
                    phases.len() - 1
                });
                at.insert(id.clone(), (p, phases[p].steps.len()));
                phases[p].steps.push(Step { label: field("label").unwrap_or_else(|| id.clone()), id, state: State::Running, result: None });
            }
            other => {
                let Some(&(p, s)) = at.get(&id) else { continue };
                let step = &mut phases[p].steps[s];
                let result = event.get("result");
                let failed = other != "result" || result.is_some_and(|r| r.get("error").is_some());
                step.state = if failed { State::Failed } else { State::Done };
                step.result = result.map(|r| r.as_str().map(str::to_owned).unwrap_or_else(|| r.to_string())).map(|r| first_line(&r));
            }
        }
    }
    let states = outcome.and_then(|o| o.get("workflowProgress")?.as_array()).into_iter().flatten().filter_map(|p| Some((p.get("agentId")?.as_str()?, p.get("state")?.as_str()?)));
    for (id, state) in states {
        if let Some(&(p, s)) = at.get(id) {
            phases[p].steps[s].state = match state {
                "done" => State::Done,
                "running" | "queued" => State::Running,
                _ => State::Failed,
            };
        }
    }
    phases
}

/// The name of a workflow and its script, from the file `<name>-<run>.js` written when it was launched.
fn script_of(scripts: &Path, run: &str) -> Option<(String, String)> {
    let suffix = format!("-{run}.js");
    std::fs::read_dir(scripts).ok()?.flatten().find_map(|entry| {
        let file = entry.file_name().to_string_lossy().into_owned();
        let name = file.strip_suffix(&suffix)?.to_owned();
        Some((name, std::fs::read_to_string(entry.path()).unwrap_or_default()))
    })
}

/// The `description` of a workflow script's `meta`: the first string after `description:`.
fn script_description(script: &str) -> Option<String> {
    let rest = script.split_once("description:")?.1.trim_start();
    let quote = rest.chars().next().filter(|c| matches!(c, '\'' | '"' | '`'))?;
    let text = rest[1..].split(quote).next()?;
    (!text.is_empty()).then(|| text.to_owned())
}

/// The last text a subagent said, from the end of its transcript.
fn last_reply(transcript: &Path) -> Option<String> {
    let (tail, cut) = crate::agent::read_tail(transcript)?;
    let whole = if cut { tail.split_once('\n').map_or("", |(_, rest)| rest) } else { &tail };
    whole.lines().rev().find_map(|line| {
        let entry: serde_json::Value = serde_json::from_str(line).ok()?;
        if entry.get("type")?.as_str()? != "assistant" {
            return None;
        }
        let text = entry.pointer("/message/content")?.as_array()?.iter().filter_map(|c| c.get("text")?.as_str()).collect::<Vec<_>>().join(" ");
        let text = first_line(&text);
        (!text.is_empty()).then_some(text)
    })
}

fn first_line(text: &str) -> String {
    text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or_default().to_owned()
}

/// What ran alongside what is running now: the running subagents and workflows, and every one that ended after the
/// earliest of them started, and so on back. Nothing when nothing runs.
fn batch(mut agents: Vec<Agent>, mut workflows: Vec<Workflow>) -> Subagents {
    let running = |finished: &Option<SystemTime>, live: bool| finished.is_none() && live;
    let Some(mut since) = agents
        .iter()
        .filter(|a| a.finished.is_none())
        .map(|a| a.started)
        .chain(workflows.iter().filter(|w| running(&w.finished, w.count(State::Running) > 0 || w.phases.is_empty())).map(|w| w.started))
        .min()
    else {
        return Subagents::default();
    };
    // An interval that ended after `since` overlaps the batch and may start it earlier.
    loop {
        let earlier = agents
            .iter()
            .filter_map(|a| Some((a.started, a.finished?)))
            .chain(workflows.iter().filter_map(|w| Some((w.started, w.finished?))))
            .filter(|&(start, end)| end >= since && start < since)
            .map(|(start, _)| start)
            .min();
        match earlier {
            Some(start) => since = start,
            None => break,
        }
    }
    agents.retain(|a| a.finished.is_none_or(|end| end >= since));
    workflows.retain(|w| w.finished.is_none_or(|end| end >= since));
    agents.sort_by_key(|a| (a.finished.is_some(), a.started));
    workflows.sort_by_key(|w| w.started);
    Subagents { agents, workflows }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn at(s: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(s)
    }

    fn agent(id: &str, start: u64, end: Option<u64>) -> Agent {
        Agent { id: id.into(), kind: "Explore".into(), title: id.into(), started: at(start), finished: end.map(at), reply: None }
    }

    #[test]
    fn the_batch_is_what_runs_and_what_ran_alongside_it() {
        let ids = |s: &Subagents| s.agents.iter().map(|a| a.id.clone()).collect::<Vec<_>>();
        // `old` ended before anything running started; `mid` overlapped `early`, which overlaps the running one.
        let got = batch(vec![agent("old", 0, Some(10)), agent("early", 20, Some(50)), agent("mid", 15, Some(25)), agent("run", 40, None)], vec![]);
        assert_eq!(ids(&got), ["run", "mid", "early"], "running first, then by start");
        assert_eq!((got.running(), got.done()), (1, 2));
        assert_eq!(batch(vec![agent("a", 0, Some(10))], vec![]), Subagents::default(), "nothing shows when nothing runs");
    }

    #[test]
    fn a_meta_file_gives_a_subagent_and_leaves_workflow_agents_to_their_journal() {
        let plain = serde_json::json!({"agentType":"general-purpose","description":"say hi","finished":true});
        let got = parse_meta("a1", &plain, at(5), Some(at(9))).unwrap();
        assert_eq!((got.kind.as_str(), got.title.as_str(), got.finished), ("general-purpose", "say hi", Some(at(9))));
        let running = serde_json::json!({"agentType":"Explore","description":"look"});
        assert_eq!(parse_meta("a2", &running, at(5), Some(at(6))).unwrap().finished, None);
        let step = serde_json::json!({"agentType":"workflow-subagent","description":"beta","workflowPhase":"Beta"});
        assert_eq!(parse_meta("a3", &step, at(5), None), None);
    }

    #[test]
    fn the_journal_gives_the_phases_and_the_outcome_their_failures() {
        let journal = [
            r#"{"type":"launched"}"#,
            r#"{"type":"started","agentId":"a","label":"scope","phase":"Scope"}"#,
            r#"{"type":"result","agentId":"a","result":{"angles":[1]}}"#,
            r#"{"type":"started","agentId":"b","label":"x","phase":"Search"}"#,
            r#"{"type":"started","agentId":"c","label":"y","phase":"Search"}"#,
            r#"{"type":"result","agentId":"b","result":"ok\nmore"}"#,
        ]
        .join("\n");
        let phases = parse_journal(&journal, None);
        assert_eq!(phases.iter().map(|p| (p.title.as_str(), p.steps.len())).collect::<Vec<_>>(), [("Scope", 1), ("Search", 2)]);
        assert_eq!(phases[1].steps.iter().map(|s| s.state).collect::<Vec<_>>(), [State::Done, State::Running]);
        assert_eq!(phases[1].steps[0].result.as_deref(), Some("ok"), "the first line");
        let outcome = serde_json::json!({"workflowProgress":[{"type":"workflow_agent","agentId":"c","state":"failed"}]});
        assert_eq!(parse_journal(&journal, Some(&outcome))[1].steps[1].state, State::Failed);
    }

    #[test]
    fn a_workflow_script_gives_its_description() {
        let script = "export const meta = {\n  name: 'alpha-beta-ok',\n  description: 'Two phases, one agent each',\n  phases: [],\n}";
        assert_eq!(script_description(script).as_deref(), Some("Two phases, one agent each"));
        assert_eq!(script_description("export const meta = { name: 'x' }"), None);
    }

    #[test]
    fn the_files_claude_code_writes_are_read_into_a_batch() {
        let root = std::env::temp_dir().join(format!("nekominal-subagents-{}", crate::agent::new_session()));
        let sub = root.join("subagents");
        let run = sub.join("workflows").join("wf_1");
        std::fs::create_dir_all(&run).unwrap();
        std::fs::create_dir_all(root.join("workflows").join("scripts")).unwrap();
        std::fs::write(sub.join("agent-p1.meta.json"), r#"{"agentType":"Explore","description":"Trace the pipe"}"#).unwrap();
        std::fs::write(sub.join("agent-p2.meta.json"), r#"{"agentType":"Explore","description":"Read the test","finished":true}"#).unwrap();
        std::fs::write(sub.join("agent-p2.jsonl"), r#"{"type":"assistant","message":{"content":[{"type":"text","text":"It races on a pipe name.\nDetails"}]}}"#).unwrap();
        std::fs::write(run.join("journal.jsonl"), "{\"type\":\"started\",\"agentId\":\"w1\",\"label\":\"a\",\"phase\":\"Write\"}\n").unwrap();
        std::fs::write(root.join("workflows").join("scripts").join("cats-wf_1.js"), "meta = { description: 'Three stories' }").unwrap();
        let got = read(&root, &Subagents::default());
        assert_eq!(got.agents.iter().map(|a| (a.title.as_str(), a.finished.is_some())).collect::<Vec<_>>(), [("Trace the pipe", false), ("Read the test", true)]);
        assert_eq!(got.agents[1].reply.as_deref(), Some("It races on a pipe name."));
        assert_eq!((got.workflows[0].name.as_str(), got.workflows[0].summary.as_deref()), ("cats", Some("Three stories")));
        assert_eq!((got.running(), got.done()), (2, 1));
        let _ = std::fs::remove_dir_all(&root);
    }
}
