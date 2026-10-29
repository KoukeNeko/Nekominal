//! The context card: under the context meter of the title bar while the pointer is on it, what fills the window of the
//! focused pane's agent (read from the cache, written to it, new input, free) and what its session has cost. Only what
//! Claude Code reports is shown: its status line when it ran, the transcript otherwise, never a guessed breakdown.

use super::*;

const WIDTH: f32 = 340.0;
const PAD: f32 = 16.0;
const ROW: f32 = 22.0;
const BAR: f32 = 8.0;
/// The Remote Control row: its name and state, and two small buttons.
const REMOTE_ROW: f32 = 32.0;

/// What the card says, from what the agent of the pane reported.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Facts {
    pub model: Option<String>,
    /// The share of the window in use, 0 to 100.
    pub share: f64,
    pub window: Option<u64>,
    /// The tokens of the window: read from the cache, written to it, new input.
    pub parts: Option<[u64; 3]>,
    pub last_out: Option<u64>,
    pub cost: Option<f64>,
    pub duration_ms: Option<u64>,
    pub api_ms: Option<u64>,
    pub totals: Option<(u64, u64)>,
    pub lines: Option<(u64, u64)>,
    /// The status line ran: without it what only it knows (cost, time) is missing for a reason the card says.
    pub status_line: bool,
    /// The session's Remote Control id while it is connected.
    pub remote: Option<String>,
    /// Where Claude Code compacts, in tokens, when its settings name a window smaller than this one.
    pub compact_at: Option<u64>,
}

impl Facts {
    pub fn of(usage: &agent::Usage) -> Option<Facts> {
        let share = usage.context?;
        let parts = match (usage.cache_read_tokens, usage.cache_creation_tokens, usage.input_tokens) {
            (None, None, None) => None,
            (read, write, new) => Some([read.unwrap_or(0), write.unwrap_or(0), new.unwrap_or(0)]),
        };
        Some(Facts {
            model: usage.model.clone(),
            share,
            window: usage.context_window,
            parts,
            last_out: usage.output_tokens,
            cost: usage.cost,
            duration_ms: usage.duration_ms,
            api_ms: usage.api_duration_ms,
            totals: usage.total_input_tokens.zip(usage.total_output_tokens),
            lines: usage.lines_added.zip(usage.lines_removed),
            // Only the status line reports a cost: from the transcript there is none.
            status_line: usage.cost.is_some(),
            remote: usage.remote.clone(),
            compact_at: usage.auto_compact.filter(|&at| usage.context_window.is_some_and(|window| at < window)),
        })
    }

    /// The tokens in use: the three parts, or the share of the window.
    pub fn used(&self) -> Option<u64> {
        self.parts.map(|p| p.iter().sum()).or_else(|| self.window.map(|w| (w as f64 * self.share / 100.0).round() as u64))
    }
}

/// A count of tokens as people read it: `950`, `2.3k`, `580.2k`, `1M`, `12.4M`.
pub fn tokens(n: u64) -> String {
    let short = |v: f64, unit: &str| {
        let text = format!("{v:.1}");
        format!("{}{unit}", text.strip_suffix(".0").unwrap_or(&text))
    };
    match n {
        0..1_000 => n.to_string(),
        1_000..1_000_000 => short(n as f64 / 1_000.0, "k"),
        _ => short(n as f64 / 1_000_000.0, "M"),
    }
}

impl App {
    pub(super) fn paint_context_card(&mut self) {
        // Open while the pointer is on the meter or on the card, so that its buttons can be reached.
        let shown = matches!(self.hover, Hit::Meter | Hit::Card | Hit::RemoteOpen | Hit::RemoteCopy);
        (self.card_rect, self.remote_buttons) = (None, None);
        if !shown {
            return;
        }
        let (Some(meter), Some(facts)) = (self.meter_rect, self.focused().and_then(|e| e.usage.as_ref()).and_then(Facts::of)) else { return };
        let g = self.glass;
        let lang = self.language();
        let (text, quiet) = (col(g.text()), col(g.text_secondary()));
        let small = self.ui.line_height(SMALL_SIZE);
        let line = self.ui.line_height(TEXT_SIZE);
        let session: Vec<(Text, String)> = [
            facts.cost.map(|c| (Text::CardCost, format!("${c:.2}"))),
            facts.duration_ms.map(|ms| (Text::CardTime, match facts.api_ms { Some(api) => format!("{} · API {}", agents_panel::clock(ms), agents_panel::clock(api)), None => agents_panel::clock(ms) })),
            facts.totals.map(|(i, o)| (Text::CardTokens, format!("{} · {}", Text::CardIn.format(lang, &[&tokens(i)]), Text::CardOut.format(lang, &[&tokens(o)])))),
            facts.last_out.map(|o| (Text::CardLastReply, Text::CardOut.format(lang, &[&tokens(o)]))),
        ]
        .into_iter()
        .flatten()
        .collect();
        let rows = if facts.parts.is_some() { 4 } else { 0 };
        let mut height = PAD + line + 10.0 + 30.0 + 10.0 + BAR + 12.0 + rows as f32 * ROW;
        if facts.compact_at.is_some() {
            height += small + 8.0;
        }
        if !session.is_empty() || facts.lines.is_some() {
            height += 12.0 + small + 6.0 + (session.len() + usize::from(facts.lines.is_some())) as f32 * ROW;
        }
        if !facts.status_line {
            height += 8.0 + 2.0 * small;
        }
        if facts.remote.is_some() {
            height += 12.0 + REMOTE_ROW;
        }
        height += PAD - 4.0;
        // Where every card of the bar hangs.
        let (card, gap) = bar_card(meter, (WIDTH, height), self.layout.size);
        self.card_rect = Some(gap);
        if self.panels_failed {
            self.ui.shadow(card, CORNER_RADIUS, 14.0, Color([0, 0, 0, 110]));
        }
        self.ui.rrect(card, CORNER_RADIUS, self.panel_fill(252), Some((col(g.divider()), 1.0)));
        self.panel_shapes.push((card, CORNER_RADIUS));
        let (x, right) = (card.x + PAD, card.x + card.w - PAD);
        let mut y = card.y + PAD;

        // Context, the model, the window.
        let w = self.ui.text(x, y, Text::CardContext.get(lang), TEXT_SIZE, text, true);
        if let Some(model) = &facts.model {
            self.ui.text(x + w + 8.0, y + (line - small) / 2.0, model, SMALL_SIZE, quiet, false);
        }
        if let Some(window) = facts.window {
            let label = Text::CardWindow.format(lang, &[&tokens(window)]);
            let lw = self.ui.measure(&label, SMALL_SIZE, false);
            self.ui.text(right - lw, y + (line - small) / 2.0, &label, SMALL_SIZE, quiet, false);
        }
        y += line + 10.0;

        // The share in large type, coloured as the meter is, and the tokens of the window.
        let share = format!("{:.0}%", facts.share);
        let big = self.ui.line_height(22.0);
        let tone = if facts.share > 70.0 { meter_color(g, facts.share) } else { text };
        let sw = self.ui.text(x, y, &share, 22.0, tone, true);
        self.ui.text(x + sw + 6.0, y + big - line - 2.0, Text::CardUsed.get(lang), TEXT_SIZE, quiet, false);
        if let (Some(used), Some(window)) = (facts.used(), facts.window) {
            let label = format!("{} / {}", tokens(used), tokens(window));
            let lw = self.ui.measure(&label, TEXT_SIZE, false);
            self.ui.text(right - lw, y + big - line - 2.0, &label, TEXT_SIZE, quiet, false);
        }
        y += 30.0 + 10.0;

        // The bar of the window, split by what fills it when that is known.
        let track = Rect::new(x, y, right - x, BAR);
        self.ui.rrect(track, BAR / 2.0, col(g.divider()), None);
        let colors = [col(g.accent), col(g.mark(g.warning)), col(g.mark(g.cyan))];
        match (facts.parts, facts.window) {
            (Some(parts), Some(window)) if window > 0 => {
                self.ui.push_clip(Rect::new(track.x, track.y, track.w, track.h));
                let mut at = track.x;
                for (part, color) in parts.iter().zip(colors) {
                    let w = track.w * (*part as f32 / window as f32).min(1.0);
                    if w > 0.0 {
                        self.ui.rrect(Rect::new(at, track.y, w.max(2.0), BAR), BAR / 2.0, color, None);
                        at += w + 2.0;
                    }
                }
                self.ui.pop_clip();
            }
            _ => self.ui.rrect(Rect::new(track.x, track.y, track.w * (facts.share as f32 / 100.0).clamp(0.0, 1.0), BAR), BAR / 2.0, meter_color(g, facts.share), None),
        }
        // Where Claude Code compacts: a tick across the bar, and how far it is under it.
        if let (Some(at), Some(window)) = (facts.compact_at, facts.window) {
            let tick = track.x + track.w * at as f32 / window as f32;
            self.ui.fill(Rect::new(tick - 0.75, track.y - 3.0, 1.5, BAR + 6.0), text);
            let label = match facts.used() {
                Some(used) if used < at => Text::CardCompactLeft.format(lang, &[&tokens(at), &tokens(at - used)]),
                _ => Text::CardCompactAt.format(lang, &[&tokens(at)]),
            };
            self.ui.text(x, y + BAR + 6.0, &label, SMALL_SIZE, quiet, false);
            y += small + 8.0;
        }
        y += BAR + 12.0;

        // A row each: its colour, its name, its tokens and its share of the window.
        if let (Some(parts), Some(window)) = (facts.parts, facts.window) {
            let free = window.saturating_sub(parts.iter().sum());
            let rows = [(Text::CardCacheRead, parts[0], colors[0]), (Text::CardCacheWrite, parts[1], colors[1]), (Text::CardNewInput, parts[2], colors[2]), (Text::CardFree, free, col(g.divider()))];
            for (name, n, color) in rows {
                let mid = y + ROW / 2.0;
                self.ui.rrect(Rect::new(x, mid - 5.0, 10.0, 10.0), 2.0, color, None);
                self.ui.text(x + 19.0, mid - line / 2.0, name.get(lang), TEXT_SIZE, text, false);
                let pct = format!("{:.1}%", n as f64 * 100.0 / window.max(1) as f64);
                let pw = self.ui.measure(&pct, TEXT_SIZE, false);
                self.ui.text(right - pw, mid - line / 2.0, &pct, TEXT_SIZE, quiet, false);
                let count = tokens(n);
                let cw = self.ui.measure(&count, TEXT_SIZE, false);
                self.ui.text(right - 56.0 - cw, mid - line / 2.0, &count, TEXT_SIZE, text, false);
                y += ROW;
            }
        }

        // What the session has cost, as far as it was reported.
        if !session.is_empty() || facts.lines.is_some() {
            y += 6.0;
            self.ui.fill(Rect::new(x, y, right - x, 1.0), col(g.divider()));
            y += 6.0;
            self.ui.text(x, y, Text::CardSession.get(lang), SMALL_SIZE, quiet, false);
            y += small + 6.0;
            for (name, value) in &session {
                let mid = y + ROW / 2.0;
                self.ui.text(x, mid - line / 2.0, name.get(lang), TEXT_SIZE, quiet, false);
                let vw = self.ui.measure(value, TEXT_SIZE, false);
                self.ui.text(right - vw, mid - line / 2.0, value, TEXT_SIZE, text, false);
                y += ROW;
            }
            if let Some((added, removed)) = facts.lines {
                let mid = y + ROW / 2.0;
                self.ui.text(x, mid - line / 2.0, Text::CardLines.get(lang), TEXT_SIZE, quiet, false);
                let (plus, minus) = (format!("+{added}"), format!("−{removed}"));
                let mw = self.ui.measure(&minus, TEXT_SIZE, false);
                self.ui.text(right - mw, mid - line / 2.0, &minus, TEXT_SIZE, col(g.mark(g.danger)), false);
                let pw = self.ui.measure(&plus, TEXT_SIZE, false);
                self.ui.text(right - mw - 8.0 - pw, mid - line / 2.0, &plus, TEXT_SIZE, col(g.mark(g.success)), false);
                y += ROW;
            }
        }
        if !facts.status_line {
            y += 8.0;
            self.ui.push_clip(Rect::new(x, y, right - x, 2.0 * small + 2.0));
            for (n, row) in wrap(&mut self.ui, Text::CardNoStatusLine.get(lang), SMALL_SIZE, right - x, 2).iter().enumerate() {
                self.ui.text(x, y + n as f32 * small, row, SMALL_SIZE, quiet, false);
            }
            self.ui.pop_clip();
            y += 2.0 * small;
        }
        // Remote Control while it is connected: its state, and its link to open or copy.
        if facts.remote.is_some() {
            y += 6.0;
            self.ui.fill(Rect::new(x, y, right - x, 1.0), col(g.divider()));
            y += 6.0;
            let mid = y + REMOTE_ROW / 2.0;
            let name_w = self.ui.text(x, mid - line / 2.0, Text::CardRemote.get(lang), TEXT_SIZE, text, false);
            let dot = Rect::new(x + name_w + 10.0, mid - 3.0, 6.0, 6.0);
            self.ui.rrect(dot, 3.0, col(g.mark(g.success)), None);
            self.ui.text(dot.x + 12.0, mid - small / 2.0, Text::CardRemoteOn.get(lang), SMALL_SIZE, col(g.mark(g.success)), false);
            let labels = [Text::CommonOpen.get(lang), Text::CardCopyLink.get(lang)];
            let widths = labels.map(|label| self.ui.measure(label, SMALL_SIZE, false) + 20.0);
            let copy = Rect::new(right - widths[1], mid - 13.0, widths[1], 26.0);
            let open = Rect::new(copy.x - 6.0 - widths[0], mid - 13.0, widths[0], 26.0);
            for ((rect, label), hit) in [open, copy].into_iter().zip(labels).zip([Hit::RemoteOpen, Hit::RemoteCopy]) {
                widgets::button(&mut self.ui, &g, rect, State { hover: self.hover == hit, pressed: self.pressed == hit, chosen: false });
                let lw = self.ui.measure(label, SMALL_SIZE, false);
                self.ui.text(rect.x + (rect.w - lw) / 2.0, mid - small / 2.0, label, SMALL_SIZE, text, false);
            }
            self.remote_buttons = Some([open, copy]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_read_as_people_say_them() {
        assert_eq!([0, 950, 2_300, 36_900, 580_200, 1_000_000, 12_400_000].map(tokens), ["0", "950", "2.3k", "36.9k", "580.2k", "1M", "12.4M"]);
        assert_eq!(tokens(999_950), "1000k", "rounding up inside a unit stays in it");
    }

    #[test]
    fn the_card_has_facts_only_with_a_context_and_uses_the_parts_before_the_share() {
        assert_eq!(Facts::of(&agent::Usage::default()), None, "no context, no card");
        let usage = agent::Usage { model: Some("Opus 5.5".into()), context: Some(58.0), ..Default::default() };
        let facts = Facts::of(&usage).unwrap();
        assert!(!facts.status_line && facts.used().is_none(), "without a window or parts the tokens are not guessed");
        let windowed = Facts { window: Some(1_000_000), ..facts.clone() };
        assert_eq!(windowed.used(), Some(580_000));
        let split = Facts { parts: Some([541_000, 36_900, 2_300]), ..windowed };
        assert_eq!(split.used(), Some(580_200));
        let from_transcript = agent::Usage { context: Some(24.5), cache_creation_tokens: Some(48_916), input_tokens: Some(2), output_tokens: Some(4), context_window: Some(200_000), ..Default::default() };
        let facts = Facts::of(&from_transcript).unwrap();
        assert_eq!((facts.parts, facts.window, facts.last_out, facts.status_line), (Some([0, 48_916, 2]), Some(200_000), Some(4), false), "a part not reported is none of the window");
    }

    /// The bar of a window too narrow for all it says: the numbers go, then the ports, before the folder.
    #[test]
    fn the_pieces_that_matter_least_give_way_first_and_the_folder_stays() {
        use super::super::{fitting, PIECE_GAP};
        // A port badge, the state, the model, the meter, the subagents, the branch, the folder: as the bar orders them.
        let ranks = [2, 5, 1, 5, 4, 3, 4];
        let widths = [40.0; 7];
        let all = 7.0 * (40.0 + PIECE_GAP);
        assert_eq!(fitting(&ranks, &widths, all), [true; 7], "room for all");
        assert_eq!(fitting(&ranks, &widths, all - 1.0), [true, true, false, true, true, true, true], "the model first");
        assert_eq!(fitting(&ranks, &widths, all - (40.0 + PIECE_GAP) - 1.0), [false, true, false, true, true, true, true], "then the port");
        assert_eq!(fitting(&ranks, &widths, 4.0 * (40.0 + PIECE_GAP)), [false, true, false, true, true, false, true], "the branch before the folder");
        assert_eq!(fitting(&ranks, &widths, 0.0), [false; 7], "no room, nothing");
        let same = [3, 3];
        assert_eq!(fitting(&same, &[40.0, 40.0], 60.0), [true, false], "of a rank, the rightmost first");
    }

    #[test]
    fn the_compaction_window_is_marked_only_inside_the_window_of_the_model() {
        let usage = agent::Usage { context: Some(22.0), context_window: Some(1_000_000), auto_compact: Some(400_000), ..Default::default() };
        assert_eq!(Facts::of(&usage).unwrap().compact_at, Some(400_000));
        assert_eq!(super::super::compact_mark(&usage), Some(0.4), "four tenths along the meter");
        let wider = agent::Usage { context_window: Some(200_000), ..usage.clone() };
        assert_eq!((Facts::of(&wider).unwrap().compact_at, super::super::compact_mark(&wider)), (None, None), "a window past the model's is never reached");
        let unknown = agent::Usage { context_window: None, ..usage };
        assert_eq!((Facts::of(&unknown).unwrap().compact_at, super::super::compact_mark(&unknown)), (None, None), "without the model's window there is nowhere to put it");
    }
}
