//! The first start: before anything else the window asks where the tabs go, with a card a layout (tabs on top, or a
//! sidebar as a list or as icons) and a button that keeps the choice. Pure: where the parts are and what a click does; the
//! window draws them.

use nekominal_model::settings::TabLayout;
use nekominal_ui::Rect;

use crate::layout::TITLE_HEIGHT;

const CARD_WIDTH: f32 = 340.0;
const CARD_GAP: f32 = 18.0;
pub const CARD_HEIGHT: f32 = 300.0;
/// The miniature of a window at the top of a card, and the space around it.
pub const MINI_HEIGHT: f32 = 150.0;
pub const CARD_PAD: f32 = 14.0;
const TITLE: f32 = 30.0;
const LEAD: f32 = 20.0;
const BUTTON: (f32, f32) = (140.0, 36.0);
/// The style of the sidebar: its label, then a button for the list and one for the icons.
const STYLE_LABEL: f32 = 44.0;
const SEGMENT: (f32, f32) = (56.0, 26.0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Top,
    Side,
    List,
    Icons,
    Start,
}

/// What the screen has chosen so far and what the pointer is on.
#[derive(Clone, Debug, PartialEq)]
pub struct Welcome {
    pub layout: TabLayout,
    pub hover: Option<Part>,
}

impl Default for Welcome {
    fn default() -> Self {
        Welcome { layout: TabLayout::Top, hover: None }
    }
}

impl Welcome {
    /// A click on `part`. True when it keeps the choice: the screen is done.
    pub fn click(&mut self, part: Part) -> bool {
        match part {
            Part::Top => self.layout = TabLayout::Top,
            Part::Side if self.layout == TabLayout::Top => self.layout = TabLayout::List,
            Part::Side => {}
            Part::List => self.layout = TabLayout::List,
            Part::Icons => self.layout = TabLayout::Icons,
            Part::Start => return true,
        }
        false
    }
}

/// Where the parts of the screen are in a window of `width` × `height` DIP: everything below the title bar, centred.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Geometry {
    pub title_y: f32,
    pub lead_y: f32,
    pub top: Rect,
    pub side: Rect,
    pub style_label: Rect,
    pub list: Rect,
    pub icons: Rect,
    pub start: Rect,
}

pub fn geometry(width: f32, height: f32) -> Geometry {
    let card_width = CARD_WIDTH.min(((width - 32.0 - CARD_GAP) / 2.0).max(160.0));
    let block = TITLE + 8.0 + LEAD + 28.0 + CARD_HEIGHT + 28.0 + BUTTON.1;
    let y = (TITLE_HEIGHT + (height - TITLE_HEIGHT - block) / 2.0).max(TITLE_HEIGHT + 16.0);
    let left = (width - 2.0 * card_width - CARD_GAP) / 2.0;
    let cards_y = y + TITLE + 8.0 + LEAD + 28.0;
    let top = Rect::new(left, cards_y, card_width, CARD_HEIGHT);
    let side = Rect::new(left + card_width + CARD_GAP, cards_y, card_width, CARD_HEIGHT);
    let style_y = side.y + side.h - CARD_PAD - SEGMENT.1;
    let style_label = Rect::new(side.x + CARD_PAD, style_y, STYLE_LABEL, SEGMENT.1);
    let list = Rect::new(style_label.x + STYLE_LABEL, style_y, SEGMENT.0, SEGMENT.1);
    let icons = Rect::new(list.x + SEGMENT.0 + 4.0, style_y, SEGMENT.0, SEGMENT.1);
    let start = Rect::new((width - BUTTON.0) / 2.0, cards_y + CARD_HEIGHT + 28.0, BUTTON.0, BUTTON.1);
    Geometry { title_y: y, lead_y: y + TITLE + 8.0, top, side, style_label, list, icons, start }
}

impl Geometry {
    pub fn hit(&self, x: f32, y: f32) -> Option<Part> {
        [(Part::List, self.list), (Part::Icons, self.icons), (Part::Start, self.start), (Part::Top, self.top), (Part::Side, self.side)]
            .into_iter()
            .find(|(_, rect)| rect.contains(x, y))
            .map(|(part, _)| part)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cards_sit_side_by_side_under_the_bar_with_the_button_below_them() {
        for (w, h) in [(1180.0, 760.0), (640.0, 480.0)] {
            let g = geometry(w, h);
            assert!(g.title_y >= TITLE_HEIGHT && g.top.x >= 0.0 && g.side.x + g.side.w <= w, "{w}×{h}");
            assert!(g.top.x + g.top.w < g.side.x && g.top.y == g.side.y);
            assert!(g.start.y > g.side.y + g.side.h);
            assert!(g.icons.x + g.icons.w <= g.side.x + g.side.w, "the style fits in its card");
        }
    }

    #[test]
    fn a_click_picks_a_layout_and_the_button_keeps_it() {
        let g = geometry(1180.0, 760.0);
        let mut w = Welcome::default();
        let at = |r: Rect| (r.x + r.w / 2.0, r.y + r.h / 2.0);
        let (x, y) = at(g.side);
        assert!(!w.click(g.hit(x, y).unwrap()));
        assert_eq!(w.layout, TabLayout::List, "the sidebar starts as a list");
        let (x, y) = at(g.icons);
        assert_eq!(g.hit(x, y), Some(Part::Icons), "the style is hit before the card it is in");
        w.click(Part::Icons);
        w.click(Part::Side);
        assert_eq!(w.layout, TabLayout::Icons, "a click on its card keeps the style");
        w.click(Part::Top);
        assert_eq!(w.layout, TabLayout::Top);
        let (x, y) = at(g.start);
        assert!(w.click(g.hit(x, y).unwrap()));
    }
}
