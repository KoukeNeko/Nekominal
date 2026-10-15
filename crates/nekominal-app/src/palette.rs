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

use crate::chip::{TabId, status_color, status_glyph, status_label, status_text};
use crate::glass::Glass;
use crate::i18n::{localize, t, tf};
use crate::keyboard::{VK_DOWN, VK_ESCAPE, VK_RETURN, VK_UP};
use crate::logic::{self, Heading, TabState, TabStatus};
use crate::shell::{Overlay, Shell};
use crate::util::*;

const PALETTE_XAML: &str = include_str!("xaml/palette.xaml");

/// A row of the palette: the label of a command and its shortcut. The text of a row is set with `SetText`, never
/// written into the markup, so what a title or a folder contains cannot change how a row loads.
const COMMAND_ROW_XAML: &str = r#"
<Grid xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation" xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml"
      HorizontalAlignment="Stretch" ColumnSpacing="16">
  <Grid.ColumnDefinitions><ColumnDefinition Width="*"/><ColumnDefinition Width="Auto"/></Grid.ColumnDefinitions>
  <TextBlock x:Name="Label" TextTrimming="CharacterEllipsis"/>
  <TextBlock x:Name="Shortcut" Grid.Column="1" Opacity="0.6" FontSize="12" VerticalAlignment="Center"/>
</Grid>"#;

/// A row of the list of every tab: status mark, number, title (with what the tab asks under it), folder, pane count
/// or the pane that waits, and shortcut. The first row of a group has the heading of the group above it.
const TAB_ROW_XAML: &str = r#"
<Grid xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation" xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml"
      HorizontalAlignment="Stretch" ColumnSpacing="12">
  <Grid.RowDefinitions>
    <RowDefinition Height="Auto"/><RowDefinition Height="Auto"/>
  </Grid.RowDefinitions>
  <Grid.ColumnDefinitions>
    <ColumnDefinition Width="12"/><ColumnDefinition Width="Auto" MinWidth="20"/><ColumnDefinition Width="*"/>
    <ColumnDefinition Width="Auto"/><ColumnDefinition Width="Auto"/><ColumnDefinition Width="Auto"/>
  </Grid.ColumnDefinitions>
  <TextBlock x:Name="Heading" Grid.ColumnSpan="6" FontSize="12" Opacity="0.6" Margin="0,0,0,4" Visibility="Collapsed"/>
  <Grid Grid.Row="1" VerticalAlignment="Center">
    <Ellipse x:Name="Dot" Width="8" Height="8" HorizontalAlignment="Center"/>
    <TextBlock x:Name="Glyph" Visibility="Collapsed" FontFamily="Segoe Fluent Icons, Segoe MDL2 Assets" FontSize="12"
               HorizontalAlignment="Center"/>
  </Grid>
  <TextBlock x:Name="Ordinal" Grid.Row="1" Grid.Column="1" Opacity="0.6" FontSize="12" VerticalAlignment="Center"/>
  <StackPanel Grid.Row="1" Grid.Column="2" VerticalAlignment="Center">
    <TextBlock x:Name="Title" TextTrimming="CharacterEllipsis"/>
    <TextBlock x:Name="Detail" Opacity="0.6" FontSize="12" TextTrimming="CharacterEllipsis" Visibility="Collapsed"/>
  </StackPanel>
  <TextBlock x:Name="Folder" Grid.Row="1" Grid.Column="3" Opacity="0.6" FontSize="12" VerticalAlignment="Center"/>
  <TextBlock x:Name="Panes" Grid.Row="1" Grid.Column="4" Opacity="0.6" FontSize="12" VerticalAlignment="Center"/>
  <TextBlock x:Name="Shortcut" Grid.Row="1" Grid.Column="5" Opacity="0.6" FontSize="12" VerticalAlignment="Center"/>
</Grid>"#;

/// The list item for a row, stretched over the width of the list, with the accessible name and shortcut the row shows.
fn list_item(content: &FrameworkElement, name: &str, shortcut: &str) -> Result<ListViewItem> {
    let item = ListViewItem::new()?;
    item.cast::<ContentControl>()?.SetContent(content)?;
    item.cast::<Control>()?.SetHorizontalContentAlignment(HorizontalAlignment::Stretch)?;
    AutomationProperties::SetName(&item, name)?;
    AutomationProperties::SetAcceleratorKey(&item, shortcut)?;
    Ok(item)
}

fn command_item(entry: &PaletteItem) -> Result<ListViewItem> {
    let row = load_xaml(COMMAND_ROW_XAML)?;
    find::<TextBlock>(&row, "Label")?.SetText(&entry.label)?;
    find::<TextBlock>(&row, "Shortcut")?.SetText(&entry.shortcut)?;
    list_item(&row, &entry.label, &entry.shortcut)
}

fn tab_item(entry: &TabRow, glass: Option<Glass>, heading: Option<Heading>) -> Result<ListViewItem> {
    let row = load_xaml(TAB_ROW_XAML)?;
    // Where the response is needed, when it is one pane of several; else how many panes there are.
    let panes = match entry.waiting_pane {
        Some((position, count)) => tf(Text::TabPanePosition, &[&position, &count]),
        None if entry.panes > 1 => tf(Text::TabPaneCount, &[&entry.panes]),
        None => String::new(),
    };
    // What the tab says when it asks for a response (an agent's question, a notification), cleaned like any title.
    let detail = entry.status.message.as_deref().filter(|_| entry.asks()).map(logic::clean_title).unwrap_or_default();
    for (name, text) in [
        ("Ordinal", entry.ordinal.to_string()),
        ("Title", entry.title.clone()),
        ("Detail", detail.clone()),
        ("Folder", entry.folder.clone()),
        ("Panes", panes),
        ("Shortcut", entry.shortcut.clone()),
    ] {
        find::<TextBlock>(&row, name)?.SetText(&text)?;
    }
    ui(&find::<TextBlock>(&row, "Detail")?)?.SetVisibility(visibility(!detail.is_empty()))?;
    if entry.unread {
        find::<TextBlock>(&row, "Title")?.SetFontWeight(FontWeight { weight: 600 })?;
    }
    if let Some(heading) = heading {
        let block = find::<TextBlock>(&row, "Heading")?;
        match status_label(heading.state, heading.attention) {
            Some(word) => block.SetText(t(word))?,
            // The tabs at rest have no word: their group only starts after a gap.
            None => fe(&block)?.SetHeight(6.0)?,
        }
        ui(&block)?.SetVisibility(Visibility::Visible)?;
    }
    if let Some(glass) = glass
        && let Err(e) = paint_state(&row, glass, entry.status.state)
    {
        log(&format!("tab row mark: {e}"));
    }
    // The state is in the name as it is in the name of a chip, so it never rests on the colour or the glyph alone.
    let name = match status_text(&entry.status) {
        Some(word) => tf(Text::TabNameStatus, &[&entry.title, &word]),
        None => entry.title.clone(),
    };
    list_item(&row, &name, &entry.shortcut)
}

/// The state of the tab: a glyph that differs from the others without its colour, or the dot.
fn paint_state(row: &FrameworkElement, glass: Glass, state: TabState) -> Result<()> {
    let brush = solid_brush(status_color(glass, state, false))?;
    let dot = find::<Shape>(row, "Dot")?;
    match status_glyph(state) {
        Some(glyph) => {
            let mark = find::<TextBlock>(row, "Glyph")?;
            mark.SetText(glyph)?;
            mark.SetForeground(&brush)?;
            ui(&mark)?.SetVisibility(Visibility::Visible)?;
            ui(&dot)?.SetVisibility(Visibility::Collapsed)?;
        }
        None => dot.SetFill(&brush)?,
    }
    Ok(())
}

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
    pub status: TabStatus,
    /// The response the tab needs has not been looked at (`logic::Watch`).
    pub unread: bool,
    /// Which pane of the tab waits for the user, as (position, count), when the tab has several.
    pub waiting_pane: Option<(usize, usize)>,
    /// The shortcut that selects the tab, when it has one.
    pub shortcut: String,
}

impl TabRow {
    /// The tab asks the user for a response: it is blocked, failed or notified.
    fn asks(&self) -> bool {
        logic::is_urgent(self.status.state) || self.status.state == TabState::Attention
    }
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
        // The list opens on the first row of the response that waits when some tab has one nobody looked at (Enter
        // goes to it), else on the tab that is shown.
        let current = shell.selected_tab().map(|tab| tab.id);
        let unread = self.shown_tabs.borrow().iter().any(|row| row.unread);
        let opened_on =
            if unread { Some(0) } else { self.shown_tabs.borrow().iter().position(|row| Some(row.id) == current) };
        if let Some(position) = opened_on {
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
        let kept = logic::keep_built(matches, |entry| {
            command_item(entry).and_then(|item| items.Append(&item)).map_err(|e| log(&format!("palette row: {e}"))).is_ok()
        });
        let count = kept.len();
        *self.shown.borrow_mut() = kept;
        count
    }

    fn refill_tabs(&self, query: &str) -> usize {
        let matches = fuzzy::rank(self.all_tabs.borrow().clone(), query, |row| row.search.as_str());
        // Nothing typed and some response unread: the rows fall into groups by state. A typed query ranks by match.
        let grouped = query.trim().is_empty() && matches.iter().any(|row| row.unread);
        let states: Vec<_> = matches.iter().map(|row| (row.status.state, row.status.attention)).collect();
        let entries: Vec<(TabRow, Option<Heading>)> = logic::list_order(&states, grouped)
            .into_iter()
            .map(|(index, heading)| (matches[index].clone(), heading))
            .collect();
        let Ok(items) = self.list.items() else { return 0 };
        let _ = items.Clear();
        let glass = self.shell.upgrade().map(|shell| shell.glass.get());
        let kept = logic::keep_built(entries, |(entry, heading)| {
            tab_item(entry, glass, *heading)
                .and_then(|item| items.Append(&item))
                .map_err(|e| log(&format!("tab row: {e}")))
                .is_ok()
        });
        let count = kept.len();
        *self.shown_tabs.borrow_mut() = kept.into_iter().map(|(row, _)| row).collect();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_markup_of_a_row_carries_no_text_of_its_own() {
        for markup in [COMMAND_ROW_XAML, TAB_ROW_XAML] {
            assert!(!markup.contains(" Text="), "the text of a row is set with SetText");
            assert!(!markup.contains("{}"), "the markup is not a format string");
        }
    }
}
