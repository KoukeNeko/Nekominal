//! The settings drawer: a glass sheet that floats over the live terminal at the right of the window. It does not
//! resize the terminal (the panes sit in a grid the sheet only covers), does not take the keyboard away from the tab
//! strip or the panes, and closes with Esc. Its rows come from `nekominal_model::settings_ui::TABLE`; this file is
//! the sheet itself (open, close, sections, search, footer) and `drawer_rows.rs` builds the controls.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use nekominal_model::commands::Command;
use nekominal_model::settings_ui::{self, Row, Section};
use nekominal_model::strings::Text;
use nekominal_model::themes;
use nekominal_winui_sys::*;
use windows_core::{EventRevoker, Interface, Result};

use crate::drawer_rows::{self, SettingRow};
use crate::i18n::{localize, t};
use crate::keyboard::{VK_END, VK_ESCAPE, VK_HOME, VK_LEFT, VK_RIGHT};
use crate::logic::{self, Arrow, Peek};
use crate::shell::Shell;
use crate::util::*;

const DRAWER_XAML: &str = include_str!("xaml/drawer.xaml");

pub struct Drawer {
    pub root: FrameworkElement,
    pub(crate) shell: Weak<Shell>,
    panel: Border,
    pub(crate) search: TextBox,
    chips: Vec<(Section, ToggleButton)>,
    pub(crate) scroll: ScrollViewer,
    rows_panel: StackPanel,
    empty: TextBlock,
    current: Cell<Section>,
    pub(crate) rows: RefCell<Vec<Rc<SettingRow>>>,
    /// The shortcuts page and the read-only rows of the advanced page, built when the sheet is first opened.
    pub(crate) extras: RefCell<drawer_rows::Extras>,
    /// Set while the controls are brought in line with the settings, so that does not count as a change.
    pub(crate) syncing: Cell<bool>,
    open: Cell<bool>,
    /// The keyboard focus is somewhere in the sheet.
    focus_inside: Cell<bool>,
    pub(crate) peek: RefCell<Peek>,
    /// The scheme the terminals show while a tile is pointed at, when it is not the one in the settings.
    peeking: RefCell<Option<&'static str>>,
    revokers: RefCell<Vec<EventRevoker>>,
}

impl Drawer {
    pub fn build(shell: &Rc<Shell>) -> Result<Rc<Self>> {
        let root = load_xaml(&localize(DRAWER_XAML))?;
        let panel: Border = root.cast()?;
        let sections: StackPanel = find(&root, "DrawerSections")?;
        let mut chips = Vec::new();
        for section in Section::ALL {
            let chip = drawer_rows::toggle_button("NkChip", t(section.text()))?;
            AutomationProperties::SetName(&chip, t(section.text()))?;
            sections.cast::<Panel>()?.Children()?.Append(&chip)?;
            chips.push((section, chip));
        }
        let drawer = Rc::new(Self {
            panel: panel.clone(),
            search: find(&root, "DrawerSearch")?,
            scroll: find(&root, "DrawerScroll")?,
            rows_panel: find(&root, "DrawerRows")?,
            empty: find(&root, "DrawerEmpty")?,
            root: root.clone(),
            shell: Rc::downgrade(shell),
            chips,
            current: Cell::new(Section::Appearance),
            rows: RefCell::new(Vec::new()),
            extras: RefCell::new(drawer_rows::Extras::default()),
            syncing: Cell::new(false),
            open: Cell::new(false),
            focus_inside: Cell::new(false),
            peek: RefCell::new(Peek::default()),
            peeking: RefCell::new(None),
            revokers: RefCell::new(Vec::new()),
        });
        AutomationProperties::SetName(&panel, t(Text::AppSettings))?;
        AutomationProperties::SetName(&drawer.search, t(Text::SettingsSearch))?;
        let close: Button = find(&root, "DrawerClose")?;
        AutomationProperties::SetName(&close, t(Text::CommonClose))?;
        ToolTipService::SetToolTip(&close, &windows_reference::IReference::from(t(Text::CommonClose)))?;

        let mut revokers = Vec::new();
        let this = drawer.clone();
        revokers.push(close.on_click(cb!("drawer close", |_, _| this.close()))?);
        for (name, command) in
            [("DrawerOpenFile", Command::AppOpenSettingsFile), ("DrawerReload", Command::AppReloadSettings)]
        {
            let button: Button = find(&root, name)?;
            let this = drawer.clone();
            revokers.push(button.on_click(cb!("drawer footer", |_, _| {
                if let Some(shell) = this.shell.upgrade() {
                    shell.run(command);
                }
            }))?);
        }
        let this = drawer.clone();
        revokers.push(drawer.search.TextChanged(cb!("drawer search", |_, _| this.refresh_view()))?);

        // Esc steps back one thing at a time; the rest of the keys are the controls' own.
        let this = drawer.clone();
        revokers.push(ui(&root)?.PreviewKeyDown(cb!("drawer key", |_, args| {
            let Some(args) = args.as_ref() else { return };
            if args.Key().map_or(0, |key| key.0) == VK_ESCAPE {
                let _ = args.SetHandled(true);
                this.escape();
            }
        }))?);
        let this = drawer.clone();
        revokers.push(ui(&root)?.GotFocus(cb!("drawer focus in", |_, _| this.focus_inside.set(true)))?);
        let this = drawer.clone();
        revokers.push(ui(&root)?.LostFocus(cb!("drawer focus out", |_, _| this.focus_inside.set(false)))?);

        // The sections: a chip picks one; the arrow keys walk the strip and pick as they go.
        let focused = Rc::new(Cell::new(0usize));
        for (index, (section, chip)) in drawer.chips.iter().enumerate() {
            let (this, section) = (drawer.clone(), *section);
            revokers.push(chip.on_click(cb!("drawer section", |_, _| this.show_section(section)))?);
            let focused = focused.clone();
            revokers.push(ui(chip)?.GotFocus(cb!("drawer section focus", |_, _| focused.set(index)))?);
        }
        let this = drawer.clone();
        revokers.push(ui(&sections)?.PreviewKeyDown(cb!("drawer section key", |_, args| {
            let Some(args) = args.as_ref() else { return };
            let arrow = match args.Key().map_or(0, |key| key.0) {
                VK_LEFT => Arrow::Left,
                VK_RIGHT => Arrow::Right,
                VK_HOME => Arrow::Home,
                VK_END => Arrow::End,
                _ => return,
            };
            let _ = args.SetHandled(true);
            if let Some(target) = logic::roving_target(focused.get(), this.chips.len(), this.chips.len(), arrow) {
                let (section, chip) = &this.chips[target];
                this.show_section(*section);
                let _ = ui(chip).and_then(|chip| chip.Focus(FocusState::Keyboard));
            }
        }))?);

        // The sheet is as wide as the window lets it be.
        let this = drawer.clone();
        let width = fe(shell.body())?.SizeChanged(cb!("drawer width", |_, args| {
            if let Some(size) = args.as_ref().and_then(|args| args.NewSize().ok()) {
                this.fit(f64::from(size.width));
            }
        }))?;
        revokers.push(width);
        drawer.fit(fe(shell.body()).and_then(|body| body.ActualWidth()).unwrap_or(0.0));

        *drawer.revokers.borrow_mut() = revokers;
        drawer_rows::build(&drawer, shell)?;
        drawer.refresh_view();
        Ok(drawer)
    }

    /// Gives the sheet the width a body of `body_width` DIP of window allows.
    fn fit(&self, body_width: f64) {
        let _ = fe(&self.panel).and_then(|panel| panel.SetWidth(logic::drawer_width(body_width)));
    }

    pub fn is_open(&self) -> bool {
        self.open.get()
    }

    /// The keyboard focus is in the sheet.
    pub fn has_focus(&self) -> bool {
        self.focus_inside.get()
    }

    /// Shows the sheet and puts the focus in the search box, or on the row `target` names (a palette entry).
    pub fn open(&self, target: Option<&str>) {
        self.open.set(true);
        let _ = ui(&self.root).and_then(|root| root.SetVisibility(Visibility::Visible));
        self.sync();
        let row = target.and_then(|id| self.rows.borrow().iter().find(|row| row.descriptor.id == id).cloned());
        match row {
            Some(row) => {
                let _ = self.search.SetText("");
                self.show_section(row.descriptor.section);
                self.focus_row(&row);
            }
            None => self.focus_search(),
        }
        log(&format!("drawer opened at {}", target.unwrap_or("the search box")));
    }

    pub fn focus_search(&self) {
        let _ = ui(&self.search).and_then(|search| search.Focus(FocusState::Programmatic));
        let _ = self.search.SelectAll();
    }

    /// Moves the focus to the control of a row and brings the row into view.
    fn focus_row(&self, row: &SettingRow) {
        // The rows were collapsed or re-parented a moment ago: they must be laid out before they can take the focus.
        let _ = ui(&self.root).and_then(|root| root.UpdateLayout());
        let _ = row.root.cast::<UIElement>().and_then(|root| root.StartBringIntoView());
        let _ = row.focus.Focus(FocusState::Programmatic);
    }

    /// Hides the sheet. The focus goes back to the pane it came from.
    pub fn close(&self) {
        if !self.open.replace(false) {
            return;
        }
        self.end_peek();
        self.stop_recording();
        let _ = self.search.SetText("");
        let _ = ui(&self.root).and_then(|root| root.SetVisibility(Visibility::Collapsed));
        self.focus_inside.set(false);
        if let Some(shell) = self.shell.upgrade() {
            shell.focus_active_pane();
        }
        log("drawer closed");
    }

    /// Esc: a key that is being recorded is let go, a scheme that is only previewed is put back, a search is cleared,
    /// and only then does the sheet close.
    fn escape(&self) {
        if self.stop_recording() {
            return;
        }
        if self.peek.borrow().tile().is_some() || self.peeking.borrow().is_some() {
            self.restore_committed_tile();
            return;
        }
        if !self.search.Text().unwrap_or_default().is_empty() {
            let _ = self.search.SetText("");
            self.focus_search();
            return;
        }
        self.close();
    }

    /// Lets go of a shortcut that is being recorded; true when there was one.
    fn stop_recording(&self) -> bool {
        self.extras.borrow().stop_recording()
    }

    /// The colour scheme tile of the settings gets the focus back, which ends the preview.
    fn restore_committed_tile(&self) {
        self.end_peek();
        let committed = self.shell.upgrade().map(|shell| shell.settings.borrow().theme.clone());
        let tile = self.rows.borrow().iter().find_map(|row| row.committed_tile(committed.as_deref()));
        if let Some(tile) = tile {
            let _ = ui(&tile).and_then(|tile| tile.Focus(FocusState::Keyboard));
        }
    }

    /// Previews nothing: the terminals show the scheme of the settings again.
    pub(crate) fn end_peek(&self) {
        self.peek.borrow_mut().clear();
        if self.peeking.take().is_some()
            && let Some(shell) = self.shell.upgrade()
        {
            shell.peek_theme(None);
        }
    }

    /// Shows the preview of the tile the pointer or the focus is on, once per frame.
    pub fn tick(&self) {
        if !self.open.get() {
            return;
        }
        let Some(shell) = self.shell.upgrade() else { return };
        let committed = shell.settings.borrow().theme.clone();
        let wanted = self
            .peek
            .borrow()
            .tile()
            .and_then(|tile| themes::theme_names().nth(tile))
            .filter(|name| *name != committed);
        if wanted == *self.peeking.borrow() {
            return;
        }
        *self.peeking.borrow_mut() = wanted;
        shell.peek_theme(wanted);
    }

    pub fn show_section(&self, section: Section) {
        self.current.set(section);
        let _ = self.search.SetText("");
        self.refresh_view();
        let _ = self.scroll.ChangeView(None, Some(0.0), None);
    }

    /// Fills the sheet with the rows of the section that is chosen, or with what the search finds.
    pub(crate) fn refresh_view(&self) {
        let Some(shell) = self.shell.upgrade() else { return };
        let Ok(children) = self.rows_panel.cast::<Panel>().and_then(|panel| panel.Children()) else { return };
        let _ = children.Clear();
        let query = self.search.Text().unwrap_or_default();
        let searching = !query.trim().is_empty();
        let mut count = 0;
        let rows = self.rows.borrow();
        if searching {
            let hits = settings_ui::search(&query, shell.lang(), &shell.keymap.borrow());
            for hit in &hits {
                let shown = match hit {
                    Row::Setting(descriptor) => {
                        rows.iter().find(|row| row.descriptor == *descriptor).map(|row| row.root.clone())
                    }
                    Row::Key(command) => self.extras.borrow().key_row(*command),
                };
                if let Some(element) = shown
                    && let Ok(element) = element.cast::<UIElement>()
                {
                    let _ = children.Append(&element);
                    count += 1;
                }
            }
        } else {
            let section = self.current.get();
            for row in rows.iter().filter(|row| row.descriptor.section == section) {
                if let Ok(element) = row.root.cast::<UIElement>() {
                    let _ = children.Append(&element);
                    count += 1;
                }
            }
            for element in self.extras.borrow().section_rows(section) {
                if let Ok(element) = element.cast::<UIElement>() {
                    let _ = children.Append(&element);
                    count += 1;
                }
            }
        }
        let _ = ui(&self.empty).and_then(|empty| empty.SetVisibility(visibility(searching && count == 0)));
        for (section, chip) in &self.chips {
            let _ = chip.SetIsChecked(Some(!searching && *section == self.current.get()));
        }
        // The chip of the section is brought into view, so a long row of them (English) never hides the chosen one.
        if !searching && let Some((_, chip)) = self.chips.iter().find(|(section, _)| *section == self.current.get()) {
            let _ = ui(chip).and_then(|chip| chip.StartBringIntoView());
        }
        drop(rows);
        self.sync();
    }

    /// Brings every control in line with the settings.
    pub fn sync(&self) {
        let Some(shell) = self.shell.upgrade() else { return };
        if self.syncing.replace(true) {
            return;
        }
        drawer_rows::sync(self, &shell);
        self.syncing.set(false);
    }
}
