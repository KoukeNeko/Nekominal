//! Command palette: an in-tree overlay (veil + glass panel with a TextBox and a ListView), not a
//! ContentDialog (modal chrome with its own buttons and key handling). The entries come from the model:
//! every command, a "new tab" entry per profile and one per colour scheme.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use nekominal_model::palette::{self as model, PaletteItem};
use windows_core::{EventRevoker, Interface, Result};

use nekominal_winui_sys::*;

use crate::i18n::localize;
use crate::keyboard::{VK_DOWN, VK_ESCAPE, VK_RETURN, VK_UP};
use crate::shell::{Overlay, Shell};
use crate::util::*;

const PALETTE_XAML: &str = include_str!("xaml/palette.xaml");

pub struct Palette {
    pub root: FrameworkElement,
    panel: Border,
    input: TextBox,
    list: ListView,
    empty: TextBlock,
    shell: Weak<Shell>,
    all: RefCell<Vec<PaletteItem>>,
    shown: RefCell<Vec<PaletteItem>>,
    revokers: RefCell<Vec<EventRevoker>>,
}

impl Palette {
    pub fn build(shell: &Rc<Shell>) -> Result<Rc<Self>> {
        let root = load_xaml(&localize(PALETTE_XAML))?;
        let palette = Rc::new(Self {
            panel: find(&root, "PalettePanel")?,
            input: find(&root, "PaletteInput")?,
            list: find(&root, "PaletteList")?,
            empty: find(&root, "PaletteEmpty")?,
            root: root.clone(),
            shell: Rc::downgrade(shell),
            all: RefCell::new(Vec::new()),
            shown: RefCell::new(Vec::new()),
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

    pub fn open(&self, shell: &Rc<Shell>) {
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
        *self.all.borrow_mut() = model::items(shell.lang(), &shell.keymap.borrow(), &shell.profiles.borrow());
        let _ = self.input.SetText("");
        self.refill();
        let _ = ui(&self.input).and_then(|input| input.Focus(FocusState::Programmatic));
    }

    fn refill(&self) {
        let query = self.input.Text().unwrap_or_default();
        let matches = model::search(self.all.borrow().clone(), &query);
        let Ok(items) = self.list.items() else { return };
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
        let _ = ui(&self.empty).and_then(|empty| empty.SetVisibility(visibility(matches.is_empty())));
        if !matches.is_empty() {
            let _ = self.list.set_selected_index(0);
        }
        *self.shown.borrow_mut() = matches;
    }

    fn move_selection(&self, delta: i32) {
        let count = self.shown.borrow().len() as i32;
        if count == 0 {
            return;
        }
        let next = (self.list.selected_index().unwrap_or(0) + delta).clamp(0, count - 1);
        let _ = self.list.set_selected_index(next);
        if let Ok(item) = self.list.items().and_then(|items| items.GetAt(next as u32)) {
            let _ = self.list.cast::<ListViewBase>().and_then(|list| list.ScrollIntoView(&item));
        }
    }

    fn run_selected(&self) {
        let index = usize::try_from(self.list.selected_index().unwrap_or(-1)).ok();
        let entry = index.and_then(|index| self.shown.borrow().get(index).cloned());
        self.close();
        if let (Some(entry), Some(shell)) = (entry, self.shell.upgrade()) {
            shell.run_palette_action(entry.action);
        }
    }
}
