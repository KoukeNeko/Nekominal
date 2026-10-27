//! The rows of the sidebar as a list, in the style the settings choose (`TabStyle`): two lines and a line of the context
//! (activity), a card with what the tab last did (cards), or a line under the repository the tab is in (repo). What a
//! row adds under its first lines is measured here for the layout, and drawn here at the same places.

use super::*;
use crate::layout::{CARD_ROW, LIST_ROW, SIDE_PAD};
use nekominal_model::settings::TabStyle;

/// The line of the context under the two lines of an activity row.
const CONTEXT_LINE: f32 = 14.0;
/// Inside a card: its padding, the box of what the tab last did (two lines) and the line of its model and numbers.
const CARD_PAD: f32 = 12.0;
const PEEK: f32 = 40.0;
const FOOT: f32 = 20.0;
/// What a worktree is moved in under its repository.
const WORKTREE_INDENT: f32 = 14.0;

/// Where the parts of a card go under its first two lines, from the top of the card, and what they add to it: the box of
/// what it last did, the line of its numbers, its badges `badges` tall.
pub fn card_parts(peek: bool, foot: bool, badges: f32) -> (Option<f32>, Option<f32>, f32, f32) {
    let mut y = CARD_ROW;
    let peek = peek.then(|| { let at = y; y += PEEK + 6.0; at });
    let foot = foot.then(|| { let at = y; y += FOOT; at });
    let badges_top = y;
    y += badges;
    // The space under the last part, as over the first.
    let extra = y - CARD_ROW + if y > CARD_ROW { 4.0 } else { 0.0 };
    (peek, foot, badges_top, extra)
}

impl App {
    fn style(&self) -> TabStyle {
        self.settings.tabs.style
    }

    /// What tab `i` last did: what its agent last said, or the last command of its shell.
    fn activity(&self, i: usize) -> Option<String> {
        let entry = self.tabs.get(i)?.focused_entry()?;
        let said = entry.pane.agent_summary().and_then(|record| record.message);
        said.or_else(|| entry.pane.term.lock().last_command_line()).map(|text| text.trim().to_owned()).filter(|text| !text.is_empty())
    }

    fn usage_of(&self, i: usize) -> Option<&agent::Usage> {
        self.tabs.get(i)?.focused_entry()?.usage.as_ref()
    }

    /// The room for the badges of a row, by style: what is right of the icon of an activity row, or the inside of a card.
    fn badge_room(&self) -> f32 {
        let row = self.mode().side_width() - 2.0 * SIDE_PAD;
        match self.style() {
            TabStyle::Cards => row - 2.0 * CARD_PAD,
            _ => row - (10.0 + TAB_ICON + 10.0) - 10.0,
        }
    }

    /// What each row of the list adds under its first lines, by tab: its line of the context and its badges, or what is in
    /// its card. Nothing but the list has rows that grow, and a tab under its repository is one line.
    pub(super) fn row_extras(&mut self) -> Vec<f32> {
        if self.settings.tabs.layout != settings::TabLayout::List || self.style() == TabStyle::Repo {
            return Vec::new();
        }
        let room = self.badge_room();
        (0..self.tabs.len())
            .map(|i| {
                let badges = badges::height(&self.row_badges(i, room).into_iter().map(|(_, rect)| rect).collect::<Vec<_>>());
                let usage = self.usage_of(i).cloned();
                match self.style() {
                    TabStyle::Cards => card_parts(self.activity(i).is_some(), usage.is_some(), badges).3,
                    _ => (if usage.and_then(|u| u.context).is_some() { CONTEXT_LINE } else { 0.0 }) + badges,
                }
            })
            .collect()
    }

    /// The colour the user gave tab `i`: a bar at the left edge of its row.
    pub(super) fn tab_color_bar(&mut self, i: usize, side: Rect, row: Rect) {
        if let Some((_, (r, gr, b))) = self.tabs[i].color.and_then(|c| logic::TAB_COLORS.get(c)) {
            let bar = Rect::new(side.x + 2.0, row.y + 6.0, 3.0, (row.h - 12.0).max(4.0));
            self.ui.rrect(bar, 1.5, Color([*r, *gr, *b, 255]), None);
        }
    }

    /// A line of the context, `share` of it used, from `x` to `right` at `y`: the meter, then its share and the cost.
    fn context_line(&mut self, x: f32, right: f32, y: f32, share: f64, cost: Option<f64>) {
        let g = self.glass;
        let text = match cost {
            Some(cost) => format!("{share:.0}% \u{b7} ${cost:.2}"),
            None => format!("{share:.0}%"),
        };
        let w = self.ui.measure(&text, SMALL_SIZE - 1.0, false);
        let small = self.ui.line_height(SMALL_SIZE - 1.0);
        self.ui.text(right - w, y - small / 2.0, &text, SMALL_SIZE - 1.0, col(g.text_secondary()), false);
        let track = Rect::new(x, y - 1.5, (right - w - 8.0 - x).max(0.0), 3.0);
        let fill = col(if share > 90.0 { g.mark(g.danger) } else if share > 70.0 { g.mark(g.warning) } else { g.accent });
        self.ui.rrect(track, 1.5, col(g.divider()), None);
        self.ui.rrect(Rect::new(track.x, track.y, track.w * (share as f32 / 100.0).clamp(0.0, 1.0), 3.0), 1.5, fill, None);
    }

    /// The colour of what a tab last did: the colour of its state when it waits or failed.
    fn activity_color(&self, state: logic::TabState) -> Color {
        let g = self.glass;
        if logic::is_urgent(state) { col(status::color(g, state)) } else { col(g.text_secondary()) }
    }

    /// The second line of an activity row (what it last did, or where it is) and what is under it: the line of the context,
    /// the badges. The words of the row start at `x`.
    pub(super) fn paint_activity_rest(&mut self, i: usize, row: Rect, x: f32) {
        let state = self.tabs[i].status().state;
        let small = self.ui.line_height(SMALL_SIZE);
        let (text, color) = match self.activity(i) {
            Some(text) => (Some(text), self.activity_color(state)),
            None => (self.place_text(i), col(self.glass.text_secondary())),
        };
        if let Some(text) = text {
            self.ui.push_clip(Rect::new(x, row.y, (row.x + row.w - 10.0 - x).max(0.0), row.h));
            self.ui.text(x, row.y + LIST_ROW - 7.0 - small, &text, SMALL_SIZE, color, false);
            self.ui.pop_clip();
        }
        let mut top = row.y + LIST_ROW - 3.0;
        if let Some(usage) = self.usage_of(i).cloned()
            && let Some(share) = usage.context
        {
            self.context_line(x, row.x + row.w - 10.0, top + CONTEXT_LINE / 2.0 - 1.0, share, usage.cost);
            top += CONTEXT_LINE;
        }
        self.paint_row_badges(i, x, top, self.badge_room());
    }

    /// A tab as a card: its program, title and state, where it is, a box of what it last did, its model and numbers, and
    /// its badges.
    pub(super) fn paint_card(&mut self, i: usize, side: Rect, row: Rect, selected: bool, hover: bool) {
        let g = self.glass;
        let lang = self.language();
        let state = self.tabs[i].status().state;
        let fill = if selected || hover { col(g.hover()) } else { { let h = col(g.hover()); Color([h.0[0], h.0[1], h.0[2], h.0[3] / 2]) } };
        let edge = if selected { (col(g.accent), 1.0) } else { (col(g.divider()), 1.0) };
        self.ui.rrect(row, 8.0, fill, Some(edge));
        self.tab_color_bar(i, side, row);
        let quiet = col(g.text_secondary());
        let (line, small) = (self.ui.line_height(TEXT_SIZE), self.ui.line_height(SMALL_SIZE));
        let top = row.y + 9.0;
        let icon = Rect::new(row.x + CARD_PAD, top, TAB_ICON, TAB_ICON);
        let name_color = if logic::is_urgent(state) { col(status::color(g, state)) } else { col(g.text()) };
        self.tab_icon(i, icon, name_color);
        // The right of the first line: the close button under the pointer, the state in a pill otherwise.
        let mut right = row.x + row.w - CARD_PAD + 4.0;
        if let Some(close) = Layout::close_button(Rect::new(row.x, top - 1.0, row.w - 4.0, 18.0)).filter(|_| hover) {
            if self.hover == Hit::ChipClose(i) {
                widgets::plate(&mut self.ui, close, col(g.pressed()));
            }
            self.icon_centered(close, '\u{E711}', ICON_SIZE, quiet);
            right = close.x - 4.0;
        } else if let Some(text) = state_text(state) {
            let text = text.get(lang);
            let color = col(status::color(g, state));
            let w = self.ui.measure(text, SMALL_SIZE, false) + 16.0;
            let pill = Rect::new(right - 4.0 - w, top, w, 17.0);
            self.ui.rrect(pill, pill.h / 2.0, Color([color.0[0], color.0[1], color.0[2], 40]), None);
            self.ui.text(pill.x + 8.0, pill.y + (pill.h - small) / 2.0, text, SMALL_SIZE, color, false);
            right = pill.x - 6.0;
        }
        let x = icon.x + TAB_ICON + 8.0;
        let label = self.tab_label(i);
        self.ui.push_clip(Rect::new(x, row.y, (right - x).max(0.0), row.h));
        self.ui.text(x, top + (TAB_ICON - line) / 2.0, &label, TEXT_SIZE, name_color, true);
        self.ui.pop_clip();
        let inner = Rect::new(row.x + CARD_PAD, row.y, row.w - 2.0 * CARD_PAD, row.h);
        if let Some(place) = self.place_text(i) {
            self.ui.push_clip(inner);
            self.ui.text(x, row.y + 29.0, &place, SMALL_SIZE, quiet, false);
            self.ui.pop_clip();
        }
        let activity = self.activity(i);
        let usage = self.usage_of(i).cloned();
        let badges = self.row_badges(i, self.badge_room());
        let tall = badges::height(&badges.iter().map(|(_, rect)| *rect).collect::<Vec<_>>());
        let (peek, foot, badges_top, _) = card_parts(activity.is_some(), usage.is_some(), tall);
        if let (Some(at), Some(text)) = (peek, activity) {
            let rect = Rect::new(inner.x, row.y + at, inner.w, PEEK);
            self.ui.rrect(rect, 6.0, Color([0, 0, 0, 56]), None);
            let color = self.activity_color(state);
            let color = if color == quiet { col(g.text()) } else { color };
            self.ui.push_clip(rect);
            for (n, text) in wrap(&mut self.ui, &text, SMALL_SIZE, rect.w - 16.0).iter().take(2).enumerate() {
                self.ui.text(rect.x + 8.0, rect.y + 4.0 + n as f32 * (small + 1.0), text, SMALL_SIZE, color, false);
            }
            self.ui.pop_clip();
        }
        if let (Some(at), Some(usage)) = (foot, usage) {
            let y = row.y + at + FOOT / 2.0;
            let tiny = self.ui.line_height(SMALL_SIZE - 1.0);
            let words = [usage.model.clone(), usage.duration_ms.map(|ms| format!("{}:{:02}", ms / 60_000, ms / 1000 % 60))].into_iter().flatten().collect::<Vec<_>>().join(" \u{b7} ");
            let w = self.ui.measure(&words, SMALL_SIZE - 1.0, false);
            self.ui.text(inner.x, y - tiny / 2.0, &words, SMALL_SIZE - 1.0, quiet, false);
            match usage.context {
                Some(share) => self.context_line(inner.x + w + 10.0, inner.x + inner.w, y, share, usage.cost),
                None => {
                    if let Some(cost) = usage.cost {
                        let text = format!("${cost:.2}");
                        let cw = self.ui.measure(&text, SMALL_SIZE - 1.0, false);
                        self.ui.text(inner.x + inner.w - cw, y - tiny / 2.0, &text, SMALL_SIZE - 1.0, quiet, false);
                    }
                }
            }
        }
        for (chip, rect) in badges {
            self.paint_badge(&chip, Rect::new(inner.x + rect.x, row.y + badges_top + rect.y, rect.w, rect.h), badges::SIDE.0);
        }
    }

    /// A tab on one line under its repository: its program, branch (or title), state, badges as far as they fit, context
    /// and cost; a worktree moved in under the main folder.
    pub(super) fn paint_repo_row(&mut self, i: usize, side: Rect, row: Rect, selected: bool, hover: bool) {
        let g = self.glass;
        let state = self.tabs[i].status().state;
        if selected {
            widgets::plate(&mut self.ui, row, col(g.hover()));
        } else if hover {
            let h = col(g.hover());
            widgets::plate(&mut self.ui, row, Color([h.0[0], h.0[1], h.0[2], h.0[3] / 2]));
        }
        self.tab_color_bar(i, side, row);
        let quiet = col(g.text_secondary());
        let uid = self.tabs[i].uid;
        let worktree = self.badges.repo.get(&uid).is_some_and(|repo| repo.worktree);
        let mid = row.y + row.h / 2.0;
        let mut x = row.x + 8.0;
        if worktree {
            // A corner from the line of the repository to this one.
            self.ui.fill(Rect::new(x + 3.0, row.y - 2.0, 1.0, row.h / 2.0 + 2.0), col(g.divider()));
            self.ui.fill(Rect::new(x + 3.0, mid, 6.0, 1.0), col(g.divider()));
            x += WORKTREE_INDENT;
        }
        let name_color = if logic::is_urgent(state) { col(status::color(g, state)) } else if selected || hover { col(g.text()) } else { quiet };
        self.tab_icon(i, Rect::new(x, mid - TAB_ICON / 2.0, TAB_ICON, TAB_ICON), name_color);
        x += TAB_ICON + 8.0;
        // From the right: the close button under the pointer, or the cost and the context; then the badges that fit.
        let mut right = row.x + row.w - 8.0;
        if let Some(close) = Layout::close_button(row).filter(|_| hover) {
            if self.hover == Hit::ChipClose(i) {
                widgets::plate(&mut self.ui, close, col(g.pressed()));
            }
            self.icon_centered(close, '\u{E711}', ICON_SIZE, quiet);
            right = close.x - 4.0;
        } else if let Some(usage) = self.usage_of(i).cloned() {
            let size = SMALL_SIZE - 1.0;
            let tiny = self.ui.line_height(size);
            if let Some(cost) = usage.cost {
                let text = format!("${cost:.2}");
                let w = self.ui.measure(&text, size, false);
                self.ui.text(right - w, mid - tiny / 2.0, &text, size, quiet, false);
                right -= w + 6.0;
            }
            if let Some(share) = usage.context {
                let track = Rect::new(right - 20.0, mid - 1.5, 20.0, 3.0);
                let fill = col(if share > 90.0 { g.mark(g.danger) } else if share > 70.0 { g.mark(g.warning) } else { g.accent });
                self.ui.rrect(track, 1.5, col(g.divider()), None);
                self.ui.rrect(Rect::new(track.x, track.y, 20.0 * (share as f32 / 100.0).clamp(0.0, 1.0), 3.0), 1.5, fill, None);
                right = track.x - 6.0;
            }
        }
        if matches!(state, logic::TabState::Working | logic::TabState::Blocked) {
            right -= ICON_SIZE;
            self.state_dot(right, mid, state);
            right -= 6.0;
        }
        // The name keeps this much; the badges have what is left, the first of them first.
        let keep = 64.0;
        let chips = self.badges.chips(uid);
        let (size, h) = badges::SIDE;
        let mut placed = Vec::new();
        let mut at = x + keep;
        for chip in chips {
            let w = badges::width(&chip, |text| self.ui.measure(text, size, false));
            if at + w > right {
                break;
            }
            placed.push((chip, w));
            at += w + 4.0;
        }
        let used: f32 = placed.iter().map(|(_, w)| w + 4.0).sum();
        let mut bx = right - used + 4.0;
        for (chip, w) in placed {
            self.paint_badge(&chip, Rect::new(bx, mid - h / 2.0, w, h), size);
            bx += w + 4.0;
        }
        let name = self.tabs[i].focused_entry().and_then(|entry| entry.branch.clone()).filter(|_| self.badges.repo.contains_key(&uid)).unwrap_or_else(|| self.tab_label(i));
        let line = self.ui.line_height(TEXT_SIZE);
        self.ui.push_clip(Rect::new(x, row.y, (right - used - x).max(0.0), row.h));
        self.ui.text(x, mid - line / 2.0, &name, TEXT_SIZE, name_color, false);
        self.ui.pop_clip();
    }

    /// The heading of the tabs of a repository: its name, how many tabs, and what their agents cost together.
    pub(super) fn paint_repo_heading(&mut self, rect: Rect, group: usize) {
        let g = self.glass;
        let lang = self.language();
        let tabs = self.groups.get(group).cloned().unwrap_or_default();
        let name = tabs.first().and_then(|i| self.tabs.get(*i)).and_then(|tab| self.badges.repo.get(&tab.uid)).map_or_else(|| Text::GroupOther.get(lang).to_owned(), badges::Repo::name);
        let small = self.ui.line_height(SMALL_SIZE);
        let y = rect.y + rect.h - small - 4.0;
        let x = rect.x + 8.0;
        let w = self.ui.text(x, y, &name, SMALL_SIZE, col(g.text()), true);
        let count = Text::RepoTabs.format(lang, &[&tabs.len()]);
        self.ui.text(x + w + 8.0, y, &count, SMALL_SIZE - 1.0, col(g.text_secondary()), false);
        let cost: f64 = tabs.iter().filter_map(|i| self.usage_of(*i).and_then(|u| u.cost)).sum();
        if cost > 0.0 {
            let text = format!("${cost:.2}");
            let cw = self.ui.measure(&text, SMALL_SIZE - 1.0, false);
            self.ui.text(rect.x + rect.w - 8.0 - cw, y, &text, SMALL_SIZE - 1.0, col(g.text_secondary()), false);
        }
    }

    /// A window with its tabs in a sidebar of style `style`, in a few rectangles, as `paint_miniature` draws the layouts.
    pub(super) fn paint_style_miniature(&mut self, mini: Rect, style: TabStyle, background: Color) {
        let g = self.glass;
        let line = col(g.divider());
        let faint = col(g.hover());
        let strong = { let t = col(g.text_secondary()); Color([t.0[0], t.0[1], t.0[2], 110]) };
        let [wait, work, done] = [logic::TabState::Blocked, logic::TabState::Working, logic::TabState::Done].map(|state| col(status::color(g, state)));
        let tint = |c: Color, a: u8| Color([c.0[0], c.0[1], c.0[2], a]);
        self.ui.rrect(mini, 6.0, background, Some((line, 1.0)));
        self.ui.fill(Rect::new(mini.x, mini.y + 14.0, mini.w, 1.0), line);
        self.ui.rrect(Rect::new(mini.x + mini.w - 38.0, mini.y + 4.0, 30.0, 6.0), 3.0, tint(wait, 128), None);
        let side = (mini.w * 0.46).round();
        self.ui.fill(Rect::new(mini.x + side, mini.y + 15.0, 1.0, mini.h - 15.0), line);
        for (n, share) in [0.8, 0.55, 0.7, 0.4].into_iter().enumerate() {
            self.ui.rrect(Rect::new(mini.x + side + 9.0, mini.y + 25.0 + n as f32 * 10.0, (mini.w - side - 18.0) * share, 4.0), 2.0, faint, None);
        }
        let (x, w) = (mini.x + 6.0, side - 12.0);
        let dot = |ui: &mut Ui, x: f32, y: f32, c: Color| ui.rrect(Rect::new(x, y, 5.0, 5.0), 2.5, c, None);
        match style {
            TabStyle::Activity => {
                // Two lines and a line of the context a tab, the first one selected and waiting.
                for (n, (color, share)) in [(wait, 0.62), (work, 0.41), (work, 0.18), (done, 0.0)].into_iter().enumerate() {
                    let y = mini.y + 21.0 + n as f32 * 25.0;
                    if n == 0 {
                        self.ui.rrect(Rect::new(x, y - 3.0, w, 23.0), 3.0, faint, None);
                    }
                    dot(&mut self.ui, x + 4.0, y, color);
                    self.ui.rrect(Rect::new(x + 13.0, y, w * 0.55, 4.0), 2.0, strong, None);
                    self.ui.rrect(Rect::new(x + 13.0, y + 7.0, w * 0.75, 3.0), 1.5, if n == 0 { tint(wait, 140) } else { faint }, None);
                    if share > 0.0 {
                        self.ui.rrect(Rect::new(x + 13.0, y + 13.0, w - 17.0, 2.0), 1.0, faint, None);
                        self.ui.rrect(Rect::new(x + 13.0, y + 13.0, (w - 17.0) * share, 2.0), 1.0, col(g.accent), None);
                    }
                }
            }
            TabStyle::Cards => {
                // Two cards, each with its pill, a box of what it did and a line under it.
                for (n, color) in [wait, work].into_iter().enumerate() {
                    let card = Rect::new(x, mini.y + 21.0 + n as f32 * 44.0, w, 39.0);
                    self.ui.rrect(card, 4.0, faint, Some((if n == 0 { col(g.accent) } else { line }, 1.0)));
                    dot(&mut self.ui, card.x + 5.0, card.y + 5.0, color);
                    self.ui.rrect(Rect::new(card.x + 14.0, card.y + 5.0, card.w * 0.45, 4.0), 2.0, strong, None);
                    self.ui.rrect(Rect::new(card.x + card.w - 22.0, card.y + 4.5, 16.0, 6.0), 3.0, tint(color, 110), None);
                    self.ui.rrect(Rect::new(card.x + 5.0, card.y + 14.0, card.w - 10.0, 12.0), 2.0, Color([0, 0, 0, 70]), None);
                    self.ui.rrect(Rect::new(card.x + 5.0, card.y + 31.0, card.w * 0.5, 3.0), 1.5, faint, None);
                }
            }
            TabStyle::Repo => {
                // Repositories with their tabs under them, a worktree moved in.
                let mut y = mini.y + 21.0;
                for (n, tabs) in [&[(wait, false)][..], &[(work, false), (work, true)][..], &[(done, false)][..]].into_iter().enumerate() {
                    self.ui.rrect(Rect::new(x + 2.0, y, w * 0.38, 3.0), 1.5, strong, None);
                    y += 7.0;
                    for (k, (color, tree)) in tabs.iter().enumerate() {
                        let indent = if *tree { 7.0 } else { 0.0 };
                        if n == 0 && k == 0 {
                            self.ui.rrect(Rect::new(x, y - 2.0, w, 10.0), 2.0, faint, None);
                        }
                        dot(&mut self.ui, x + 3.0 + indent, y + 0.5, *color);
                        self.ui.rrect(Rect::new(x + 12.0 + indent, y + 1.0, w - 30.0 - indent, 3.0), 1.5, strong, None);
                        self.ui.rrect(Rect::new(x + w - 10.0, y, 6.0, 6.0), 3.0, Color::TRANSPARENT, Some((col(g.accent), 1.2)));
                        y += 11.0;
                    }
                    y += 4.0;
                }
            }
        }
    }

    /// The tabs by their repositories, in the order each repository first comes, the main folder before its worktrees;
    /// the tabs in none last.
    pub(super) fn repo_groups(&self) -> Vec<Vec<usize>> {
        let mut roots: Vec<Option<std::path::PathBuf>> = Vec::new();
        let mut groups: Vec<Vec<usize>> = Vec::new();
        let mut loose = Vec::new();
        for (i, tab) in self.tabs.iter().enumerate() {
            let Some(repo) = self.badges.repo.get(&tab.uid) else {
                loose.push(i);
                continue;
            };
            match roots.iter().position(|root| root.as_ref() == Some(&repo.root)) {
                Some(g) => groups[g].push(i),
                None => {
                    roots.push(Some(repo.root.clone()));
                    groups.push(vec![i]);
                }
            }
        }
        for group in &mut groups {
            group.sort_by_key(|i| self.badges.repo.get(&self.tabs[*i].uid).is_some_and(|repo| repo.worktree));
        }
        if !loose.is_empty() {
            groups.push(loose);
        }
        groups
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_card_holds_what_it_has_and_grows_by_as_much() {
        assert_eq!(card_parts(false, false, 0.0), (None, None, CARD_ROW, 0.0), "a bare card is its two lines");
        let (peek, foot, badges, extra) = card_parts(true, true, 20.0);
        assert_eq!((peek, foot, badges), (Some(CARD_ROW), Some(CARD_ROW + PEEK + 6.0), CARD_ROW + PEEK + 6.0 + FOOT));
        assert_eq!(extra, PEEK + 6.0 + FOOT + 20.0 + 4.0);
        let (peek, foot, _, extra) = card_parts(false, true, 0.0);
        assert_eq!((peek, foot, extra), (None, Some(CARD_ROW), FOOT + 4.0), "no box without anything done");
    }
}
