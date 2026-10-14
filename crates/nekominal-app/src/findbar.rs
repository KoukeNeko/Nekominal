//! Find bar: a glass overlay at the top right of the terminal area. It searches the scrollback of the focused
//! pane for literal text and scrolls to the match; Enter and Shift+Enter step through the matches.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use nekominal_model::strings::Text;
use nekominal_term::SearchMatch;
use nekominal_winui_sys::*;
use windows_core::{EventRevoker, Interface, Result};

use crate::i18n::{localize, t, tf};
use crate::keyboard::{self, VK_ESCAPE, VK_RETURN};
use crate::shell::{Overlay, Shell};
use crate::util::*;

const FINDBAR_XAML: &str = include_str!("xaml/findbar.xaml");

/// More matches than this are counted but not kept.
const MATCH_LIMIT: usize = 20_000;

pub struct FindBar {
    pub root: FrameworkElement,
    border: Border,
    input: TextBox,
    count: TextBlock,
    shell: Weak<Shell>,
    matches: RefCell<Vec<SearchMatch>>,
    current: Cell<usize>,
    revokers: RefCell<Vec<EventRevoker>>,
}

/// The regular expression that matches `text` literally.
fn literal_pattern(text: &str) -> String {
    let mut pattern = String::with_capacity(text.len() * 2);
    for ch in text.chars() {
        if "\\.+*?()|[]{}^$#&-~".contains(ch) {
            pattern.push('\\');
        }
        pattern.push(ch);
    }
    pattern
}

impl FindBar {
    pub fn build(shell: &Rc<Shell>) -> Result<Rc<Self>> {
        let root = load_xaml(&localize(FINDBAR_XAML))?;
        let bar = Rc::new(Self {
            border: root.cast()?,
            input: find(&root, "FindInput")?,
            count: find(&root, "FindCount")?,
            root: root.clone(),
            shell: Rc::downgrade(shell),
            matches: RefCell::new(Vec::new()),
            current: Cell::new(0),
            revokers: RefCell::new(Vec::new()),
        });
        let glass = shell.glass.get();
        let _ = bar.border.SetBackground(&solid_brush(glass.panel())?);
        let _ = bar.border.SetBorderBrush(&solid_brush(glass.panel_border())?);
        let mut revokers = Vec::new();

        let this = bar.clone();
        revokers.push(bar.input.TextChanged(cb!("find text", |_, _| this.search()))?);
        let this = bar.clone();
        revokers.push(ui(&bar.input)?.PreviewKeyDown(cb!("find key", |_, args| {
            let Some(args) = args.as_ref() else { return };
            match args.Key().map_or(0, |key| key.0) {
                VK_ESCAPE => {
                    let _ = args.SetHandled(true);
                    this.close();
                }
                VK_RETURN => {
                    let _ = args.SetHandled(true);
                    this.step(if keyboard::held_modifiers().shift { -1 } else { 1 });
                }
                _ => {}
            }
        }))?);
        for (name, delta) in [("FindPrevious", -1), ("FindNext", 1)] {
            let this = bar.clone();
            let button: Button = find(&root, name)?;
            revokers.push(button.on_click(cb!("find step", |_, _| this.step(delta)))?);
        }
        let this = bar.clone();
        let close: Button = find(&root, "FindClose")?;
        revokers.push(close.on_click(cb!("find close", |_, _| this.close()))?);
        *bar.revokers.borrow_mut() = revokers;
        Ok(bar)
    }

    pub fn show(&self, shown: bool) {
        let _ = ui(&self.root).and_then(|root| root.SetVisibility(visibility(shown)));
    }

    pub fn focus(&self) {
        let _ = ui(&self.input).and_then(|input| input.Focus(FocusState::Programmatic));
        let _ = self.input.SelectAll();
    }

    fn close(&self) {
        if let Some(shell) = self.shell.upgrade() {
            shell.set_overlay(Overlay::None);
        }
    }

    fn search(&self) {
        let text = self.input.Text().unwrap_or_default();
        let found = if text.is_empty() {
            Vec::new()
        } else {
            self.shell
                .upgrade()
                .and_then(|shell| shell.selected_tab())
                .and_then(|tab| tab.focused_pane())
                .and_then(|pane| pane.pane.term.lock().search_all(&literal_pattern(&text), MATCH_LIMIT).ok())
                .unwrap_or_default()
        };
        // Start at the match nearest the bottom of the buffer, where the output a user is after usually is.
        self.current.set(found.len().saturating_sub(1));
        *self.matches.borrow_mut() = found;
        self.reveal();
    }

    fn step(&self, delta: i32) {
        let count = self.matches.borrow().len();
        if count == 0 {
            return;
        }
        let next = (self.current.get() as i64 + i64::from(delta)).rem_euclid(count as i64);
        self.current.set(next as usize);
        self.reveal();
    }

    fn reveal(&self) {
        let matches = self.matches.borrow();
        let label = if self.input.Text().unwrap_or_default().is_empty() {
            String::new()
        } else if matches.is_empty() {
            t(Text::FindNone).to_owned()
        } else {
            tf(Text::FindCount, &[&(self.current.get() + 1), &matches.len()])
        };
        let _ = self.count.SetText(&label);
        if let (Some(found), Some(pane)) = (
            matches.get(self.current.get()),
            self.shell.upgrade().and_then(|shell| shell.selected_tab()).and_then(|tab| tab.focused_pane()),
        ) {
            pane.pane.term.lock().scroll_to_line(found.start.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::literal_pattern;

    #[test]
    fn regex_metacharacters_are_matched_literally() {
        assert_eq!(literal_pattern("a.b"), "a\\.b");
        assert_eq!(literal_pattern("(x)[1]*"), "\\(x\\)\\[1\\]\\*");
        assert_eq!(literal_pattern("路徑 C:\\dir"), "路徑 C:\\\\dir");
    }
}
