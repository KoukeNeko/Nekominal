//! The drawer a row of the subagents card opens, on the right of the window under the bar: one subagent (what it was
//! asked, the tools it called, what it said last), a workflow (its phases, the agents of the one picked), or an agent of a
//! workflow. It lies over the panes and does not resize them: a pane made narrower would make the agent in it draw its
//! whole screen again. Esc or the arrow goes back a level, the cross closes it.

use std::path::PathBuf;
use std::time::SystemTime;

use super::*;
use super::subagents_card::{Target, dot_rows, fit, since, step_color};
use crate::subagents::{Detail, State as Step};

const WIDTH: f32 = 440.0;
const PAD: f32 = 18.0;
const NAV: f32 = 40.0;
const FOOT: f32 = 48.0;
const DOT: f32 = 10.0;
const DOT_GAP: f32 = 4.0;
const PHASE_NAME: f32 = 58.0;
const PHASE_COUNT: f32 = 44.0;
/// The most agents of a phase the drawer lists, and lines of a prompt it shows.
const MOST_STEPS: usize = 8;
const PROMPT_LINES: usize = 4;

/// What a part of the drawer does when clicked.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Back,
    Close,
    Open(Target),
    Phase(usize),
    Copy(String),
    OpenFile(PathBuf),
}

/// What the drawer last read of a transcript, kept while the file does not change.
pub struct Cached {
    pub file: PathBuf,
    pub stamp: Option<SystemTime>,
    pub detail: Detail,
}

impl App {
    /// The folder of the focused pane's session, beside its transcript.
    fn session_folder(&self) -> Option<PathBuf> {
        self.focused()?.usage.as_ref()?.transcript.as_deref().map(crate::subagents::folder)
    }

    /// What a transcript says, read again only when it changed since the last frame that asked.
    fn transcript_detail(&mut self, file: PathBuf) -> Detail {
        let stamp = std::fs::metadata(&file).and_then(|m| m.modified()).ok();
        if let Some(cached) = &self.sub_detail
            && cached.file == file
            && cached.stamp == stamp
        {
            return cached.detail.clone();
        }
        let detail = crate::subagents::detail(&file);
        self.sub_detail = Some(Cached { file, stamp, detail: detail.clone() });
        detail
    }

    pub(super) fn open_drawer(&mut self, target: Target) {
        self.sub_phase = None;
        self.sub_drawer = Some(target);
        self.redraw();
    }

    /// One level up: from an agent of a workflow to the workflow, from anything else out of the drawer.
    pub(super) fn drawer_back(&mut self) {
        self.sub_drawer = match self.sub_drawer.take() {
            Some(Target::Step(run, _)) => Some(Target::Workflow(run)),
            _ => None,
        };
        self.redraw();
    }

    pub(super) fn drawer_action(&mut self, k: usize) {
        let Some((_, action)) = self.drawer_hits.get(k).cloned() else { return };
        match action {
            Action::Back => self.drawer_back(),
            Action::Close => self.sub_drawer = None,
            Action::Open(target) => self.sub_drawer = Some(target),
            Action::Phase(p) => self.sub_phase = Some(p),
            Action::Copy(text) => {
                clipboard::write_text(&text);
            }
            Action::OpenFile(file) => {
                if let Err(e) = crate::open::shell_open(&file.to_string_lossy()) {
                    util::log(&format!("open {}: {e}", file.display()));
                }
            }
        }
        self.redraw();
    }

    pub(super) fn paint_subagents_drawer(&mut self) {
        (self.drawer_rect, self.drawer_hits) = (None, Vec::new());
        let Some(target) = self.sub_drawer.clone() else { return };
        let (Some(folder), Some(subs)) = (self.session_folder(), self.focused().map(|e| e.subagents.clone())) else {
            self.sub_drawer = None;
            return;
        };
        let g = self.glass;
        let lang = self.language();
        let (width, height) = self.layout.size;
        let drawer = Rect::new(width - WIDTH, layout::TITLE_HEIGHT, WIDTH, height - layout::TITLE_HEIGHT);
        self.drawer_rect = Some(drawer);
        if self.panels_failed {
            self.ui.shadow(drawer, 0.0, 18.0, Color([0, 0, 0, 90]));
        }
        self.ui.fill(drawer, self.panel_fill(255));
        self.panel_shapes.push((drawer, 0.0));
        self.ui.fill(Rect::new(drawer.x, drawer.y, 1.0, drawer.h), col(g.divider()));
        let (text, quiet) = (col(g.text()), col(g.text_secondary()));
        let small = self.ui.line_height(SMALL_SIZE);
        let line = self.ui.line_height(TEXT_SIZE);

        // The bar of the drawer: back, the level above it, the cross.
        let up = match &target {
            Target::Step(run, _) => subs.workflows.iter().find(|w| &w.run == run).map_or_else(|| run.clone(), |w| w.name.clone()),
            _ => Text::SubagentsTitle.get(lang).to_owned(),
        };
        let back_w = self.ui.measure(&up, SMALL_SIZE, false) + 30.0;
        let back = Rect::new(drawer.x + 10.0, drawer.y + (NAV - 26.0) / 2.0, back_w, 26.0);
        let close = Rect::new(drawer.x + drawer.w - 36.0, back.y, 26.0, 26.0);
        for (rect, action) in [(back, Action::Back), (close, Action::Close)] {
            let k = self.drawer_hits.len();
            if self.hover == Hit::Drawer(k) {
                widgets::plate(&mut self.ui, rect, col(g.hover()));
            }
            self.drawer_hits.push((rect, action));
        }
        self.icon_centered(Rect::new(back.x + 2.0, back.y, 18.0, back.h), '\u{E76B}', SMALL_SIZE, quiet);
        self.ui.text(back.x + 22.0, back.y + (back.h - small) / 2.0, &up, SMALL_SIZE, quiet, false);
        self.icon_centered(close, '\u{E711}', SMALL_SIZE, quiet);
        self.ui.fill(Rect::new(drawer.x, drawer.y + NAV, drawer.w, 1.0), col(g.divider()));

        let (x, right) = (drawer.x + PAD, drawer.x + drawer.w - PAD);
        let inner = right - x;
        let bottom = drawer.y + drawer.h - FOOT;
        let mut y = drawer.y + NAV + 16.0;
        let mut foot: Vec<(String, Action)> = Vec::new();
        self.ui.push_clip(Rect::new(drawer.x, drawer.y + NAV + 1.0, drawer.w, bottom - drawer.y - NAV - 1.0));

        match &target {
            Target::Agent(id) => {
                let Some(agent) = subs.agents.iter().find(|a| &a.id == id).cloned() else {
                    self.ui.pop_clip();
                    self.sub_drawer = None;
                    return;
                };
                let state = match agent.finished {
                    None => (Step::Running, since(agent.started)),
                    Some(end) => (Step::Done, agents_panel::clock(end.duration_since(agent.started).unwrap_or_default().as_millis() as u64)),
                };
                y = self.drawer_head(x, y, &agent.kind, state, &agent.title);
                let file = crate::subagents::transcript_of(&folder, None, id);
                let detail = self.transcript_detail(file.clone());
                let reply = agent.reply.clone().or_else(|| detail.reply.clone().filter(|_| agent.finished.is_some()));
                if let Some(reply) = &reply {
                    y = self.drawer_section(x, y, inner, Text::SubagentsReply.get(lang), None, reply, PROMPT_LINES, true);
                    foot.push((Text::SubagentsCopyReply.get(lang).to_owned(), Action::Copy(reply.clone())));
                }
                if let Some(prompt) = &detail.prompt {
                    y = self.drawer_section(x, y, inner, Text::SubagentsPrompt.get(lang), None, prompt, PROMPT_LINES, false);
                }
                self.drawer_calls(x, y, inner, &detail, bottom);
                foot.push((Text::SubagentsOpenTranscript.get(lang).to_owned(), Action::OpenFile(file)));
            }
            Target::Workflow(run) => {
                let Some(workflow) = subs.workflows.iter().find(|w| &w.run == run).cloned() else {
                    self.ui.pop_clip();
                    self.sub_drawer = None;
                    return;
                };
                let running = workflow.phases.iter().flat_map(|p| &p.steps).any(|s| s.state == Step::Running);
                let time = workflow.finished.map_or_else(|| since(workflow.started), |end| agents_panel::clock(end.duration_since(workflow.started).unwrap_or_default().as_millis() as u64));
                y = self.drawer_head(x, y, Text::SubagentsWorkflow.get(lang), (if running { Step::Running } else { Step::Done }, time), &workflow.name);
                if let Some(summary) = &workflow.summary {
                    for row in wrap(&mut self.ui, summary, SMALL_SIZE, inner, 2) {
                        self.ui.text(x, y, &row, SMALL_SIZE, quiet, false);
                        y += small;
                    }
                    y += 10.0;
                }
                // The phases, as in the card but with bigger dots, each an agent to open; a phase's name picks it.
                let picked = self.sub_phase.filter(|&p| p < workflow.phases.len()).unwrap_or_else(|| {
                    workflow.phases.iter().rposition(|p| p.steps.iter().any(|s| s.state == Step::Running)).unwrap_or(workflow.phases.len().saturating_sub(1))
                });
                let strip = inner - PHASE_NAME - PHASE_COUNT;
                for (p, phase) in workflow.phases.iter().enumerate() {
                    let rows = dot_rows(phase.steps.len(), strip - 1.0) as f32;
                    let row_h = rows * (DOT + DOT_GAP) + 6.0;
                    let name = Rect::new(x - 6.0, y - 3.0, PHASE_NAME, line + 4.0);
                    let k = self.drawer_hits.len();
                    if self.hover == Hit::Drawer(k) {
                        widgets::plate(&mut self.ui, name, col(g.hover()));
                    }
                    self.drawer_hits.push((name, Action::Phase(p)));
                    let title = fit(&mut self.ui, &phase.title, SMALL_SIZE, p == picked, PHASE_NAME - 10.0);
                    self.ui.text(x, y, &title, SMALL_SIZE, if p == picked { text } else { quiet }, p == picked);
                    let (mut dx, mut dy) = (0.0, 0.0);
                    for step in &phase.steps {
                        if dx + DOT > strip {
                            (dx, dy) = (0.0, dy + DOT + DOT_GAP);
                        }
                        let dot = Rect::new(x + PHASE_NAME + dx, y + 2.0 + dy, DOT, DOT);
                        let k = self.drawer_hits.len();
                        self.ui.rrect(dot, 2.5, step_color(&g, step.state), (self.hover == Hit::Drawer(k)).then(|| (text, 1.5)));
                        self.drawer_hits.push((dot, Action::Open(Target::Step(run.clone(), step.id.clone()))));
                        dx += DOT + DOT_GAP;
                    }
                    let finished = phase.steps.iter().filter(|s| s.state != Step::Running).count();
                    let count = format!("{finished}/{}", phase.steps.len());
                    let cw = self.ui.measure(&count, SMALL_SIZE, false);
                    self.ui.text(right - cw, y, &count, SMALL_SIZE, if finished < phase.steps.len() { text } else { quiet }, false);
                    y += row_h.max(line + 4.0);
                }
                // The agents of the picked phase: running, failed, then done.
                if let Some(phase) = workflow.phases.get(picked) {
                    y += 8.0;
                    let mut steps = phase.steps.clone();
                    steps.sort_by_key(|s| match s.state {
                        Step::Running => 0,
                        Step::Failed => 1,
                        Step::Done => 2,
                    });
                    let count = |state| phase.steps.iter().filter(|s| s.state == state).count();
                    let head = format!("{} · {}", phase.title, [(Step::Running, Text::SubagentsRunning), (Step::Failed, Text::SubagentsFailedCount), (Step::Done, Text::SubagentsDone)]
                        .into_iter()
                        .filter(|(state, _)| count(*state) > 0)
                        .map(|(state, words)| words.format(lang, &[&count(state).to_string()]))
                        .collect::<Vec<_>>()
                        .join(" · "));
                    self.ui.text(x, y, &head, SMALL_SIZE, quiet, false);
                    y += small + 6.0;
                    let room = ((bottom - y) / (line + small + 8.0)).floor().max(1.0) as usize;
                    let shown = steps.len().min(MOST_STEPS).min(room);
                    for step in &steps[..shown] {
                        let rect = Rect::new(x - 6.0, y - 3.0, inner + 12.0, line + if step.result.is_some() { small + 2.0 } else { 0.0 } + 6.0);
                        let k = self.drawer_hits.len();
                        if self.hover == Hit::Drawer(k) {
                            widgets::plate(&mut self.ui, rect, col(g.hover()));
                        }
                        self.drawer_hits.push((rect, Action::Open(Target::Step(run.clone(), step.id.clone()))));
                        self.ui.rrect(Rect::new(x, y + line / 2.0 - 4.0, 8.0, 8.0), 2.0, step_color(&g, step.state), None);
                        let label = fit(&mut self.ui, &step.label, TEXT_SIZE, false, inner - 18.0);
                        self.ui.text(x + 18.0, y, &label, TEXT_SIZE, text, false);
                        y += line;
                        if let Some(result) = &step.result {
                            let result = fit(&mut self.ui, result, SMALL_SIZE, false, inner - 18.0);
                            let color = if step.state == Step::Failed { col(g.mark(g.danger)) } else { quiet };
                            self.ui.text(x + 18.0, y, &result, SMALL_SIZE, color, false);
                            y += small + 2.0;
                        }
                        y += 6.0;
                    }
                    if steps.len() > shown {
                        self.ui.text(x + 18.0, y, &Text::SubagentsMore.format(lang, &[&(steps.len() - shown).to_string()]), SMALL_SIZE, quiet, false);
                    }
                }
                foot.push((Text::SubagentsOpenJournal.get(lang).to_owned(), Action::OpenFile(crate::subagents::journal_of(&folder, run))));
            }
            Target::Step(run, id) => {
                let found = subs.workflows.iter().find(|w| &w.run == run).and_then(|w| w.phases.iter().find_map(|p| Some((p.title.clone(), p.steps.iter().find(|s| &s.id == id)?.clone()))));
                let Some((phase, step)) = found else {
                    self.ui.pop_clip();
                    self.sub_drawer = None;
                    return;
                };
                y = self.drawer_head(x, y, &phase, (step.state, String::new()), &step.label);
                let file = crate::subagents::transcript_of(&folder, Some(run), id);
                let detail = self.transcript_detail(file.clone());
                let result = step.result.clone().or_else(|| detail.reply.clone().filter(|_| step.state != Step::Running));
                if let Some(result) = &result {
                    y = self.drawer_section(x, y, inner, Text::SubagentsResult.get(lang), None, result, PROMPT_LINES, true);
                    foot.push((Text::SubagentsCopyReply.get(lang).to_owned(), Action::Copy(result.clone())));
                }
                if let Some(prompt) = &detail.prompt {
                    y = self.drawer_section(x, y, inner, Text::SubagentsPrompt.get(lang), None, prompt, PROMPT_LINES, false);
                }
                self.drawer_calls(x, y, inner, &detail, bottom);
                foot.push((Text::SubagentsOpenTranscript.get(lang).to_owned(), Action::OpenFile(file)));
            }
        }
        self.ui.pop_clip();

        // The foot: the buttons, and how to go back.
        self.ui.fill(Rect::new(drawer.x, bottom, drawer.w, 1.0), col(g.divider()));
        let mut bx = drawer.x + 14.0;
        for (label, action) in foot {
            let w = self.ui.measure(&label, SMALL_SIZE, false) + 22.0;
            let rect = Rect::new(bx, bottom + (FOOT - 28.0) / 2.0, w, 28.0);
            let k = self.drawer_hits.len();
            widgets::button(&mut self.ui, &g, rect, State { hover: self.hover == Hit::Drawer(k), pressed: self.pressed == Hit::Drawer(k), chosen: false });
            self.ui.text(rect.x + 11.0, rect.y + (28.0 - small) / 2.0, &label, SMALL_SIZE, text, false);
            self.drawer_hits.push((rect, action));
            bx += w + 6.0;
        }
        let hint = Text::SubagentsEscBack.get(lang);
        let hw = self.ui.measure(hint, SMALL_SIZE, false);
        if bx + 10.0 < right - hw {
            self.ui.text(right - hw, bottom + (FOOT - small) / 2.0, hint, SMALL_SIZE, quiet, false);
        }
    }

    /// The head of the drawer: its kind and state on one line, its title under them. Returns where what follows starts.
    fn drawer_head(&mut self, x: f32, y: f32, kind: &str, (state, time): (Step, String), title: &str) -> f32 {
        let g = self.glass;
        let lang = self.language();
        let (quiet, small, line) = (col(g.text_secondary()), self.ui.line_height(SMALL_SIZE), self.ui.line_height(15.0));
        let w = self.ui.text(x, y, kind, SMALL_SIZE, quiet, false);
        let (words, color) = match state {
            Step::Running => (Text::SubagentsRunningFor.format(lang, &[&time]), col(g.accent)),
            Step::Done if time.is_empty() => (Text::AgentDone.get(lang).to_owned(), quiet),
            Step::Done => (Text::SubagentsFinishedAfter.format(lang, &[&time]), quiet),
            // The same word as a failed event of the push settings.
            Step::Failed => (Text::PushEventFailed.get(lang).to_owned(), col(g.mark(g.danger))),
        };
        let dot = Rect::new(x + w + 12.0, y + small / 2.0 - 3.5, 7.0, 7.0);
        match state {
            Step::Done => self.ui.rrect(dot, 3.5, Color::TRANSPARENT, Some((quiet, 1.5))),
            _ => self.ui.rrect(dot, 3.5, color, None),
        }
        self.ui.text(dot.x + 12.0, y, &words, SMALL_SIZE, color, false);
        let mut y = y + small + 4.0;
        let inner = WIDTH - 2.0 * PAD;
        for row in wrap(&mut self.ui, title, 15.0, inner, 2) {
            self.ui.text(x, y, &row, 15.0, col(g.text()), true);
            y += line;
        }
        y + 14.0
    }

    /// A section: its heading and `body` in at most `lines` lines, on a lighter ground when `boxed`.
    #[allow(clippy::too_many_arguments)]
    fn drawer_section(&mut self, x: f32, y: f32, inner: f32, heading: &str, aside: Option<&str>, body: &str, lines: usize, boxed: bool) -> f32 {
        let g = self.glass;
        let (quiet, small, line) = (col(g.text_secondary()), self.ui.line_height(SMALL_SIZE), self.ui.line_height(TEXT_SIZE));
        self.ui.text(x, y, heading, SMALL_SIZE, quiet, false);
        if let Some(aside) = aside {
            let w = self.ui.measure(aside, SMALL_SIZE, false);
            self.ui.text(x + inner - w, y, aside, SMALL_SIZE, quiet, false);
        }
        let mut y = y + small + 6.0;
        let pad = if boxed { 10.0 } else { 0.0 };
        let flat = body.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" ");
        let rows = wrap(&mut self.ui, &flat, TEXT_SIZE, inner - 2.0 * pad, lines);
        if boxed {
            let h = rows.len() as f32 * line + 2.0 * pad - 4.0;
            self.ui.rrect(Rect::new(x, y, inner, h), 6.0, Color([255, 255, 255, 9]), None);
        }
        y += pad - 2.0;
        for row in rows {
            self.ui.text(x + pad, y, &row, TEXT_SIZE, col(g.text()), false);
            y += line;
        }
        y + pad + 12.0
    }

    /// The last tool calls of a transcript, the one still running lit, and how many came before them.
    fn drawer_calls(&mut self, x: f32, y: f32, inner: f32, detail: &Detail, bottom: f32) {
        if detail.total_calls == 0 {
            return;
        }
        let g = self.glass;
        let lang = self.language();
        let (quiet, small, line) = (col(g.text_secondary()), self.ui.line_height(SMALL_SIZE), self.ui.line_height(SMALL_SIZE + 0.5));
        self.ui.text(x, y, &Text::SubagentsToolCalls.format(lang, &[&detail.total_calls.to_string()]), SMALL_SIZE, quiet, false);
        let mut y = y + small + 6.0;
        let tool_w = 80.0;
        for call in &detail.calls {
            if y + line > bottom - 8.0 {
                break;
            }
            let color = if call.open { col(g.accent) } else { col(g.divider()) };
            self.ui.rrect(Rect::new(x + 1.0, y + line / 2.0 - 3.0, 6.0, 6.0), 3.0, color, None);
            let tool = fit(&mut self.ui, &call.tool, SMALL_SIZE, false, tool_w - 6.0);
            self.ui.text(x + 15.0, y, &tool, SMALL_SIZE, quiet, false);
            let arg = fit(&mut self.ui, &call.arg, SMALL_SIZE, false, inner - 15.0 - tool_w);
            self.ui.text(x + 15.0 + tool_w, y, &arg, SMALL_SIZE, col(g.text()), false);
            y += line + 4.0;
        }
        let earlier = detail.total_calls - detail.calls.len();
        if earlier > 0 && y + small < bottom - 8.0 {
            self.ui.text(x + 15.0, y, &Text::SubagentsEarlier.format(lang, &[&earlier.to_string()]), SMALL_SIZE, quiet, false);
        }
    }
}
