//! Command palette: an in-tree overlay (veil + glass panel with a TextBox and a ListView), not a
//! ContentDialog (modal chrome with its own buttons and key handling). The entries come from the model:
//! every command, a "go to tab" entry per open tab, a "new tab" entry per profile and one per colour scheme.
//!
//! The same overlay is also the list of every tab (`open_tabs`): one row per tab in strip order with its status,
//! number, title, folder, pane count and shortcut, filtered by what is typed.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Instant;

use nekominal_model::fuzzy;
use nekominal_model::palette::{self as model, PaletteAction, PaletteItem};
use nekominal_model::strings::Text;
use windows_core::{EventRevoker, Interface, Result};

use nekominal_winui_sys::*;

use crate::chip::{TabId, status_color, status_glyph};
use crate::i18n::{localize, t, tf};
use crate::keyboard::{VK_DOWN, VK_ESCAPE, VK_RETURN, VK_UP};
use crate::logic::TabState;
use crate::shell::{Overlay, Shell};
use crate::util::*;

const PALETTE_XAML: &str = include_str!("xaml/palette.xaml");

/// A tab as the list of every tab shows it.
#[derive(Clone, Debug)]
pub struct TabRow {
    pub id: TabId,
    /// Position in the strip, one-based.
    pub ordinal: usize,
    pub title: String,
    /// Last segment of the working directory; empty when the shell never reported one.
    pub folder: String,
    /// What typing in the list is matched against: the title and the folder.
    pub search: String,
    pub panes: usize,
    pub state: TabState,
    /// The shortcut that selects the tab, when it has one.
    pub shortcut: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Commands,
    Tabs,
}

pub struct Palette {
    pub root: FrameworkElement,
    panel: Border,
    title: TextBlock,
    input: TextBox,
    list: ListView,
    empty: TextBlock,
    shell: Weak<Shell>,
    mode: Cell<Mode>,
    all: RefCell<Vec<PaletteItem>>,
    shown: RefCell<Vec<PaletteItem>>,
    all_tabs: RefCell<Vec<TabRow>>,
    shown_tabs: RefCell<Vec<TabRow>>,
    revokers: RefCell<Vec<EventRevoker>>,
}

impl Palette {
    pub fn build(shell: &Rc<Shell>) -> Result<Rc<Self>> {
        let root = load_xaml(&localize(PALETTE_XAML))?;
        let palette = Rc::new(Self {
            panel: find(&root, "PalettePanel")?,
            title: find(&root, "PaletteTitle")?,
            input: find(&root, "PaletteInput")?,
            list: find(&root, "PaletteList")?,
            empty: find(&root, "PaletteEmpty")?,
            root: root.clone(),
            shell: Rc::downgrade(shell),
            mode: Cell::new(Mode::Commands),
            all: RefCell::new(Vec::new()),
            shown: RefCell::new(Vec::new()),
            all_tabs: RefCell::new(Vec::new()),
            shown_tabs: RefCell::new(Vec::new()),
            revokers: RefCell::new(Vec::new()),
        });
        let mut revokers = Vec::new();

        let this = palette.clone();
        revokers.push(palette.input.TextChanged(cb!("palette text", |_, _| this.refill()))?);

        // Up/Down move the selection, Enter runs it, Escape closes. Handled keeps the focus in the TextBox.
        let this = palette.clone();
        revokers.push(ui(&palette.input)?.PreviewKeyDown(cb!("palette key", |_, args| {
            let Some(args) = args.as_ref() else { return };
            match args.Key().map_or(0, |key| key.0) {
                code @ (VK_DOWN | VK_UP) => {
                    let _ = args.SetHandled(true);
                    this.move_selection(if code == VK_DOWN { 1 } else { -1 });
                }
                VK_RETURN => {
                    let _ = args.SetHandled(true);
                    this.run_selected();
                }
                VK_ESCAPE => {
                    let _ = args.SetHandled(true);
                    this.close();
                }
                _ => {}
            }
        }))?);

        let this = palette.clone();
        revokers.push(palette.list.cast::<ListViewBase>()?.ItemClick(cb!("palette click", |_, args| {
            let Some(args) = args.as_ref() else { return };
            let mut index = 0u32;
            let found = this
                .list
                .items()
                .and_then(|items| args.ClickedItem().and_then(|item| items.IndexOf(&item, &mut index)));
            if found.unwrap_or(false) {
                let _ = this.list.set_selected_index(index as i32);
                this.run_selected();
            }
        }))?);

        // A press on the veil itself (not on the panel) dismisses the palette.
        let (this, veil) = (palette.clone(), root.clone());
        revokers.push(ui(&root)?.PointerPressed(cb!("palette veil", |_, args| {
            let pressed_veil = args
                .as_ref()
                .and_then(|a| a.cast::<RoutedEventArgs>().ok())
                .and_then(|a| a.OriginalSource().ok())
                .is_some_and(|source| same(&source, &veil));
            if pressed_veil {
                this.close();
            }
        }))?);
        *palette.revokers.borrow_mut() = revokers;
        Ok(palette)
    }

    pub fn show(&self, shown: bool) {
        let _ = ui(&self.root).and_then(|root| root.SetVisibility(visibility(shown)));
    }

    fn close(&self) {
        if let Some(shell) = self.shell.upgrade() {
            shell.set_overlay(Overlay::None);
        }
    }

    /// The glass of the panel and the veil, from the colour scheme in effect.
    fn restyle(&self, shell: &Rc<Shell>) {
        let glass = shell.glass.get();
        if let Ok(root) = self.root.cast::<Panel>()
            && let Ok(brush) = solid_brush(glass.veil())
        {
            let _ = root.SetBackground(&brush);
        }
        if let Ok(brush) = solid_brush(glass.panel()) {
            let _ = self.panel.SetBackground(&brush);
        }
        if let Ok(brush) = solid_brush(glass.panel_border()) {
            let _ = self.panel.SetBorderBrush(&brush);
        }
    }

    /// Opens the palette of commands.
    pub fn open(&self, shell: &Rc<Shell>) {
        self.restyle(shell);
        self.mode.set(Mode::Commands);
        let _ = ui(&self.title).and_then(|title| title.SetVisibility(Visibility::Collapsed));
        let _ = self.input.SetPlaceholderText(t(Text::PalettePlaceholder));
        let _ = self.empty.SetText(t(Text::PaletteEmpty));
        *self.all.borrow_mut() =
            model::items(shell.lang(), &shell.keymap.borrow(), &shell.profiles.borrow(), &shell.palette_tabs());
        let _ = self.input.SetText("");
        self.refill();
        let _ = ui(&self.input).and_then(|input| input.Focus(FocusState::Programmatic));
    }

    /// Opens the list of every tab with the selected tab's row highlighted.
    pub fn open_tabs(&self, shell: &Rc<Shell>) {
        self.restyle(shell);
        self.mode.set(Mode::Tabs);
        let _ = ui(&self.title).and_then(|title| title.SetVisibility(Visibility::Visible));
        let _ = self.input.SetPlaceholderText(t(Text::TabListPlaceholder));
        let _ = self.empty.SetText(t(Text::TabListEmpty));
        *self.all_tabs.borrow_mut() = shell.tab_rows();
        let _ = self.input.SetText("");
        self.refill();
        let current = shell.selected_index();
        if let Some(position) = self.shown_tabs.borrow().get(current).map(|_| current) {
            self.select(position);
        }
        let _ = ui(&self.input).and_then(|input| input.Focus(FocusState::Programmatic));
    }

    fn refill(&self) {
        let query = self.input.Text().unwrap_or_default();
        let started = Instant::now();
        let count = match self.mode.get() {
            Mode::Commands => self.refill_commands(&query),
            Mode::Tabs => self.refill_tabs(&query),
        };
        let _ = ui(&self.empty).and_then(|empty| empty.SetVisibility(visibility(count == 0)));
        if count > 0 {
            let _ = self.list.set_selected_index(0);
        }
        if self.mode.get() == Mode::Tabs {
            crate::util::log(&format!("tab list: {count} rows in {:.1} ms", started.elapsed().as_secs_f64() * 1000.0));
        }
    }

    fn refill_commands(&self, query: &str) -> usize {
        let matches = model::search(self.all.borrow().clone(), query);
        let Ok(items) = self.list.items() else { return 0 };
        let _ = items.Clear();
        for entry in &matches {
            let row = format!(
                r#"<Grid xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation" HorizontalAlignment="Stretch" ColumnSpacing="16">
                     <Grid.ColumnDefinitions><ColumnDefinition Width="*"/><ColumnDefinition Width="Auto"/></Grid.ColumnDefinitions>
                     <TextBlock Text="{}" TextTrimming="CharacterEllipsis"/>
                     <TextBlock Grid.Column="1" Text="{}" Opacity="0.6" FontSize="12" VerticalAlignment="Center"/>
                   </Grid>"#,
                xml_escape(&entry.label),
                xml_escape(&entry.shortcut)
            );
            let (Ok(content), Ok(item)) = (load_xaml(&row), ListViewItem::new()) else { continue };
            let _ = item.cast::<ContentControl>().and_then(|c| c.SetContent(&content));
            let _ = item.cast::<Control>().and_then(|c| c.SetHorizontalContentAlignment(HorizontalAlignment::Stretch));
            let _ = items.Append(&item);
        }
        let count = matches.len();
        *self.shown.borrow_mut() = matches;
        count
    }

    fn refill_tabs(&self, query: &str) -> usize {
        let matches = fuzzy::rank(self.all_tabs.borrow().clone(), query, |row| row.search.as_str());
        let Ok(items) = self.list.items() else { return 0 };
        let _ = items.Clear();
        let glass = self.shell.upgrade().map(|shell| shell.glass.get());
        for entry in &matches {
            let panes = if entry.panes > 1 { tf(Text::TabPaneCount, &[&entry.panes]) } else { String::new() };
            let row = format!(
                r#"<Grid xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation" xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml"
                         HorizontalAlignment="Stretch" ColumnSpacing="12">
                     <Grid.ColumnDefinitions>
                       <ColumnDefinition Width="12"/><ColumnDefinition Width="Auto" MinWidth="20"/><ColumnDefinition Width="*"/>
                       <ColumnDefinition Width="Auto"/><ColumnDefinition Width="Auto"/><ColumnDefinition Width="Auto"/>
                     </Grid.ColumnDefinitions>
                     <Grid VerticalAlignment="Center">
                       <Ellipse x:Name="Dot" Width="8" Height="8" HorizontalAlignment="Center"/>
                       <TextBlock x:Name="Glyph" Visibility="Collapsed" FontFamily="Segoe Fluent Icons, Segoe MDL2 Assets" FontSize="12"
                                  HorizontalAlignment="Center"/>
                     </Grid>
                     <TextBlock Grid.Column="1" Text="{}" Opacity="0.6" FontSize="12" VerticalAlignment="Center"/>
                     <TextBlock Grid.Column="2" Text="{}" TextTrimming="CharacterEllipsis"/>
                     <TextBlock Grid.Column="3" Text="{}" Opacity="0.6" FontSize="12" VerticalAlignment="Center"/>
                     <TextBlock Grid.Column="4" Text="{}" Opacity="0.6" FontSize="12" VerticalAlignment="Center"/>
                     <TextBlock Grid.Column="5" Text="{}" Opacity="0.6" FontSize="12" VerticalAlignment="Center"/>
                   </Grid>"#,
                entry.ordinal,
                xml_escape(&entry.title),
                xml_escape(&entry.folder),
                xml_escape(&panes),
                xml_escape(&entry.shortcut)
            );
            let (Ok(content), Ok(item)) = (load_xaml(&row), ListViewItem::new()) else { continue };
            // The state of the tab: a glyph that differs from the others without its colour, or the dot.
            if let Some(glass) = glass
                && let Ok(brush) = solid_brush(status_color(glass, entry.state, false))
            {
                match (status_glyph(entry.state), find::<TextBlock>(&content, "Glyph"), find::<Shape>(&content, "Dot"))
                {
                    (Some(glyph), Ok(mark), Ok(dot)) => {
                        let _ = (mark.SetText(glyph), mark.SetForeground(&brush));
                        let _ = (
                            ui(&mark).and_then(|m| m.SetVisibility(Visibility::Visible)),
                            ui(&dot).and_then(|d| d.SetVisibility(Visibility::Collapsed)),
                        );
                    }
                    (_, _, Ok(dot)) => {
                        let _ = dot.SetFill(&brush);
                    }
                    _ => {}
                }
            }
            let _ = item.cast::<ContentControl>().and_then(|c| c.SetContent(&content));
            let _ = item.cast::<Control>().and_then(|c| c.SetHorizontalContentAlignment(HorizontalAlignment::Stretch));
            let _ = items.Append(&item);
        }
        let count = matches.len();
        *self.shown_tabs.borrow_mut() = matches;
        count
    }

    fn shown_count(&self) -> usize {
        match self.mode.get() {
            Mode::Commands => self.shown.borrow().len(),
            Mode::Tabs => self.shown_tabs.borrow().len(),
        }
    }

    fn select(&self, index: usize) {
        let _ = self.list.set_selected_index(index as i32);
        if let Ok(item) = self.list.items().and_then(|items| items.GetAt(index as u32)) {
            let _ = self.list.cast::<ListViewBase>().and_then(|list| list.ScrollIntoView(&item));
        }
    }

    fn move_selection(&self, delta: i32) {
        let count = self.shown_count() as i32;
        if count == 0 {
            return;
        }
        let next = (self.list.selected_index().unwrap_or(0) + delta).clamp(0, count - 1);
        self.select(next as usize);
    }

    fn run_selected(&self) {
        let index = usize::try_from(self.list.selected_index().unwrap_or(-1)).ok();
        let action = index.and_then(|index| match self.mode.get() {
            Mode::Commands => self.shown.borrow().get(index).map(|entry| entry.action.clone()),
            Mode::Tabs => self.shown_tabs.borrow().get(index).map(|row| PaletteAction::GotoTab(row.id)),
        });
        self.close();
        if let (Some(action), Some(shell)) = (action, self.shell.upgrade()) {
            shell.run_palette_action(action);
        }
    }
}
