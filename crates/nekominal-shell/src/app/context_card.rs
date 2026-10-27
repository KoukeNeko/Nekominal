//! The context card: under the context meter of the title bar while the pointer is on it, what fills the window of the
//! focused pane's agent (read from the cache, written to it, new input, free) and what its session has cost. Only what
//! Claude Code reports is shown: its status line when it ran, the transcript otherwise, never a guessed breakdown.

use super::*;

const WIDTH: f32 = 340.0;
const PAD: f32 = 16.0;
const ROW: f32 = 22.0;
const BAR: f32 = 8.0;

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
}

impl Facts {
    pub fn of(usage: &agent::Usage) -> Option<Facts> {
        let share = usage.context?;
        Some(Facts { model: usage.model.clone(), share, cost: usage.cost, duration_ms: usage.duration_ms, status_line: usage.cost.is_some(), ..Facts::default() })
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
        if self.hover != Hit::Meter {
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
        if !session.is_empty() || facts.lines.is_some() {
            height += 12.0 + small + 6.0 + (session.len() + usize::from(facts.lines.is_some())) as f32 * ROW;
        }
        if !facts.status_line {
            height += 8.0 + 2.0 * small;
        }
        height += PAD - 4.0;
        let width = self.layout.size.0;
        let mid = meter.x + meter.w / 2.0;
        let card = Rect::new((mid - 70.0).clamp(8.0, (width - WIDTH - 8.0).max(8.0)), layout::TITLE_HEIGHT + 4.0, WIDTH, height);
        self.ui.shadow(card, CORNER_RADIUS, 14.0, Color([0, 0, 0, 110]));
        self.ui.rrect(card, CORNER_RADIUS, Color([g.background.r, g.background.g, g.background.b, 252]), Some((col(g.divider()), 1.0)));
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
    }
}
