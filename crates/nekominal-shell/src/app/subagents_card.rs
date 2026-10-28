//! The subagents of the focused pane's Claude Code session in the title bar: a piece right of the context meter while one
//! runs ("3 running · 12 done"), and under it while the pointer is on it a card with every workflow (its phases as rows
//! of dots, one an agent) and every subagent (its kind, title and time; the last thing it said once it ended). A row is
//! clicked to open it in the drawer (`subagents_drawer.rs`).

use std::time::SystemTime;

use super::*;
use crate::subagents::{State as Step, Subagents};

const WIDTH: f32 = 380.0;
const PAD: f32 = 16.0;
const ROW: f32 = 24.0;
const DOT: f32 = 8.0;
const DOT_GAP: f32 = 3.0;
/// The column of a phase's name and of its count.
const PHASE_NAME: f32 = 58.0;
const PHASE_COUNT: f32 = 44.0;
/// The most subagents the card lists; the others are counted.
const MOST_AGENTS: usize = 8;

/// What a row of the card opens in the drawer: a subagent, a workflow, or an agent of a workflow (its run and id).
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    Agent(String),
    Workflow(String),
    Step(String, String),
}

/// How long ago `since` was, as the bar says times.
pub fn since(at: SystemTime) -> String {
    agents_panel::clock(SystemTime::now().duration_since(at).unwrap_or_default().as_millis() as u64)
}

/// `text` cut with an ellipsis to fit `width` at `size`.
pub fn fit(ui: &mut Ui, text: &str, size: f32, bold: bool, width: f32) -> String {
    if ui.measure(text, size, bold) <= width {
        return text.to_owned();
    }
    let mut chars: Vec<char> = text.chars().collect();
    while !chars.is_empty() {
        chars.pop();
        let cut: String = chars.iter().collect::<String>().trim_end().to_owned() + "…";
        if ui.measure(&cut, size, bold) <= width {
            return cut;
        }
    }
    String::new()
}

/// The colour of a dot of a phase.
pub fn step_color(g: &Glass, state: Step) -> Color {
    match state {
        Step::Running => col(g.accent),
        Step::Done => col(g.divider()),
        Step::Failed => col(g.mark(g.danger)),
    }
}

/// The rows of dots of a phase's steps in `width`: how many rows they take.
pub fn dot_rows(count: usize, width: f32) -> usize {
    let per_row = (((width + DOT_GAP) / (DOT + DOT_GAP)).floor() as usize).max(1);
    count.div_ceil(per_row).max(1)
}

impl App {
    /// The subagents of the focused pane, when one runs.
    pub(super) fn subagents(&self) -> Option<&Subagents> {
        self.focused().map(|e| &e.subagents).filter(|s| s.running() > 0)
    }

    pub(super) fn paint_subagents_card(&mut self) {
        let shown = matches!(self.hover, Hit::Subagents | Hit::SubCard | Hit::SubRow(_));
        (self.sub_card_rect, self.sub_rows) = (None, Vec::new());
        if !shown || self.sub_drawer.is_some() {
            return;
        }
        let (Some(piece), Some(subs)) = (self.subagents_rect, self.subagents().cloned()) else { return };
        let g = self.glass;
        let lang = self.language();
        let (text, quiet) = (col(g.text()), col(g.text_secondary()));
        let small = self.ui.line_height(SMALL_SIZE);
        let line = self.ui.line_height(TEXT_SIZE);
        let inner = WIDTH - 2.0 * PAD;
        let strip = inner - PHASE_NAME - PHASE_COUNT;
        let listed = subs.agents.len().min(MOST_AGENTS);

        let mut height = PAD + line;
        for workflow in &subs.workflows {
            height += 13.0 + line + 4.0 + if workflow.summary.is_some() { small + 6.0 } else { 0.0 };
            height += workflow.phases.iter().map(|p| dot_rows(p.steps.len(), strip) as f32 * (DOT + DOT_GAP) + 8.0).sum::<f32>();
        }
        if listed > 0 {
            height += 13.0;
            height += subs.agents[..listed].iter().map(|a| ROW + if a.reply.is_some() { small + 2.0 } else { 0.0 }).sum::<f32>();
            if subs.agents.len() > listed {
                height += small + 4.0;
            }
        }
        height += PAD - 4.0;

        let width = self.layout.size.0;
        let mid = piece.x + piece.w / 2.0;
        let card = Rect::new((mid - 90.0).clamp(8.0, (width - WIDTH - 8.0).max(8.0)), layout::TITLE_HEIGHT + 4.0, WIDTH, height);
        self.sub_card_rect = Some(Rect::new(card.x, piece.y + piece.h, card.w, card.y + card.h - (piece.y + piece.h)));
        self.ui.shadow(card, CORNER_RADIUS, 14.0, Color([0, 0, 0, 110]));
        self.ui.rrect(card, CORNER_RADIUS, Color([g.background.r, g.background.g, g.background.b, 252]), Some((col(g.divider()), 1.0)));
        let (x, right) = (card.x + PAD, card.x + card.w - PAD);
        let mut y = card.y + PAD;

        let w = self.ui.text(x, y, Text::SubagentsTitle.get(lang), TEXT_SIZE, text, true);
        self.ui.text(x + w + 8.0, y + (line - small) / 2.0, &counts(lang, &subs), SMALL_SIZE, quiet, false);
        y += line;

        let mut rows = Vec::new();
        for workflow in &subs.workflows {
            y += 6.0;
            self.ui.fill(Rect::new(x, y, inner, 1.0), col(g.divider()));
            y += 7.0;
            let top = y;
            let name = fit(&mut self.ui, &workflow.name, TEXT_SIZE, true, inner * 0.6);
            let nw = self.ui.text(x, y, &name, TEXT_SIZE, text, true);
            let time = workflow.finished.map_or_else(|| since(workflow.started), |end| agents_panel::clock(end.duration_since(workflow.started).unwrap_or_default().as_millis() as u64));
            self.ui.text(x + nw + 8.0, y + (line - small) / 2.0, &format!("{} · {time}", Text::SubagentsWorkflow.get(lang)), SMALL_SIZE, quiet, false);
            y += line + 4.0;
            if let Some(summary) = &workflow.summary {
                let summary = fit(&mut self.ui, summary, SMALL_SIZE, false, inner);
                self.ui.text(x, y, &summary, SMALL_SIZE, quiet, false);
                y += small + 6.0;
            }
            for phase in &workflow.phases {
                let title = fit(&mut self.ui, &phase.title, SMALL_SIZE, false, PHASE_NAME - 6.0);
                self.ui.text(x, y - 2.0, &title, SMALL_SIZE, quiet, false);
                let (mut dx, mut dy) = (0.0, 0.0);
                for step in &phase.steps {
                    if dx + DOT > strip {
                        (dx, dy) = (0.0, dy + DOT + DOT_GAP);
                    }
                    self.ui.rrect(Rect::new(x + PHASE_NAME + dx, y + 2.0 + dy, DOT, DOT), 2.0, step_color(&g, step.state), None);
                    dx += DOT + DOT_GAP;
                }
                let finished = phase.steps.iter().filter(|s| s.state != Step::Running).count();
                let count = format!("{finished}/{}", phase.steps.len());
                let cw = self.ui.measure(&count, SMALL_SIZE, false);
                let live = phase.steps.iter().any(|s| s.state == Step::Running);
                self.ui.text(right - cw, y - 2.0, &count, SMALL_SIZE, if live { text } else { quiet }, false);
                y += dot_rows(phase.steps.len(), strip) as f32 * (DOT + DOT_GAP) + 8.0;
            }
            rows.push((Rect::new(card.x, top - 4.0, card.w, y - top), Target::Workflow(workflow.run.clone())));
        }

        if listed > 0 {
            y += 6.0;
            self.ui.fill(Rect::new(x, y, inner, 1.0), col(g.divider()));
            y += 7.0;
            let kinds: Vec<String> = subs.agents[..listed].iter().map(|a| a.kind.clone()).collect();
            let kind_w = kinds.iter().map(|k| self.ui.measure(k, SMALL_SIZE, false)).fold(0.0, f32::max).min(inner * 0.32);
            for agent in &subs.agents[..listed] {
                let top = y;
                let mid = y + ROW / 2.0;
                let rect = Rect::new(card.x + 4.0, top, card.w - 8.0, ROW + if agent.reply.is_some() { small + 2.0 } else { 0.0 });
                let k = rows.len();
                if self.hover == Hit::SubRow(k) {
                    widgets::plate(&mut self.ui, rect, col(g.hover()));
                }
                let dot = Rect::new(x, mid - 4.0, 8.0, 8.0);
                match agent.finished {
                    None => self.ui.rrect(dot, 4.0, col(g.accent), None),
                    Some(_) => self.ui.rrect(dot, 4.0, Color([0, 0, 0, 0]), Some((col(g.divider()), 1.5))),
                }
                let kind = fit(&mut self.ui, &agent.kind, SMALL_SIZE, false, kind_w);
                self.ui.text(x + 17.0, mid - small / 2.0, &kind, SMALL_SIZE, quiet, false);
                let time = agent.finished.map_or_else(|| since(agent.started), |end| agents_panel::clock(end.duration_since(agent.started).unwrap_or_default().as_millis() as u64));
                let tw = self.ui.measure(&time, SMALL_SIZE, false);
                self.ui.text(right - tw, mid - small / 2.0, &time, SMALL_SIZE, quiet, false);
                let title_x = x + 17.0 + kind_w + 10.0;
                let title = fit(&mut self.ui, &agent.title, TEXT_SIZE, false, right - tw - 10.0 - title_x);
                self.ui.text(title_x, mid - line / 2.0, &title, TEXT_SIZE, if agent.finished.is_some() { quiet } else { text }, false);
                y += ROW;
                if let Some(reply) = &agent.reply {
                    let reply = fit(&mut self.ui, reply, SMALL_SIZE, false, right - title_x);
                    self.ui.text(title_x, y - 3.0, &reply, SMALL_SIZE, quiet, false);
                    y += small + 2.0;
                }
                rows.push((rect, Target::Agent(agent.id.clone())));
            }
            if subs.agents.len() > listed {
                let more = Text::SubagentsMore.format(lang, &[&(subs.agents.len() - listed).to_string()]);
                self.ui.text(x + 17.0, y + 2.0, &more, SMALL_SIZE, quiet, false);
            }
        }
        self.sub_rows = rows;
    }
}

/// "3 running · 12 done", or the running alone when nothing ended yet.
pub fn counts(lang: Lang, subs: &Subagents) -> String {
    let running = Text::SubagentsRunning.format(lang, &[&subs.running().to_string()]);
    match subs.done() {
        0 => running,
        done => format!("{running} · {}", Text::SubagentsDone.format(lang, &[&done.to_string()])),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dots_of_a_phase_wrap_into_rows_of_the_strip() {
        // 8 px dots, 3 px apart: 22 fit in 239 px.
        assert_eq!(dot_rows(0, 239.0), 1, "an empty phase still has its row");
        assert_eq!(dot_rows(22, 239.0), 1);
        assert_eq!(dot_rows(23, 239.0), 2);
        assert_eq!(dot_rows(76, 239.0), 4);
        assert_eq!(dot_rows(5, 1.0), 5, "one a row when nothing fits");
    }
}
