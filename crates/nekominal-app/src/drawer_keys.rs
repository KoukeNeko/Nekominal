//! The shortcuts page of the settings drawer: one row per command of the palette, with its shortcuts as buttons. A button
//! (or the plus) waits for the next chord and gives it to the command; a chord that another command has, or that a program
//! in the terminal needs (Ctrl+C, Ctrl+D, Alt+B), is not taken until the row says what it runs into and Replace is pressed.
//! The model decides what a chord clashes with and writes the bindings (`nekominal_model::keybindings`); this file is the
//! rows and the keyboard capture.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use nekominal_model::commands::Command;
use nekominal_model::keybindings::{self, Clash};
use nekominal_model::keymap::Shortcut;
use nekominal_model::settings_ui;
use nekominal_model::strings::Text;
use nekominal_winui_sys::*;
use windows_core::{EventRevoker, Interface, Result};
use windows_reference::IReference;

use crate::drawer_rows::{row_frame, stack};
use crate::i18n::{t, tf};
use crate::keyboard::{self, VK_BACK, VK_DELETE, VK_ESCAPE};
use crate::shell::Shell;
use crate::util::*;

/// What a row is doing with the keyboard.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Capture {
    Idle,
    /// Waiting for a chord; it replaces `replacing`, or is one more binding.
    Recording {
        replacing: Option<Shortcut>,
    },
    /// The chord that came is taken: the row says by what and waits for Replace or Cancel.
    Conflict {
        replacing: Option<Shortcut>,
        chord: Shortcut,
        clash: Clash,
    },
}

/// The one row that is waiting for a chord, if any: starting another cancels it.
struct Session {
    active: RefCell<Option<Rc<KeyRow>>>,
    shell: Weak<Shell>,
}

struct KeyRow {
    command: Command,
    root: FrameworkElement,
    bar: UIElement,
    reset: Button,
    chips: StackPanel,
    conflict: StackPanel,
    conflict_text: TextBlock,
    replace: Button,
    cancel: Button,
    capture: Cell<Capture>,
    /// The button that is waiting for the chord, to give it the focus.
    recorder: RefCell<Option<Button>>,
    /// The row is being drawn again: the button that had the focus is gone, which is not the user leaving.
    drawing: Cell<bool>,
    /// The keyboard focus is on a button of the row.
    inside: Cell<bool>,
    /// What the buttons show now, to see when the keymap changed under the row.
    shown: RefCell<(Vec<String>, bool)>,
    session: Rc<Session>,
    chip_revokers: RefCell<Vec<EventRevoker>>,
    _revokers: RefCell<Vec<EventRevoker>>,
}

/// The rows of the page and the button above them that restores every shortcut.
#[derive(Default)]
pub struct KeysPage {
    rows: Vec<Rc<KeyRow>>,
    restore: Option<RestoreRow>,
    session: Option<Rc<Session>>,
}

struct RestoreRow {
    root: FrameworkElement,
    button: Button,
    _revoker: EventRevoker,
}

/// A button in the colours of the sheet, small enough to hold a chord.
fn chord_button(text: &str, name: &str, accent: bool) -> Result<Button> {
    let foreground = if accent { r#" Foreground="{StaticResource NkPanelAccent}""# } else { "" };
    let markup = format!(
        r#"<Button xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation" Style="{{StaticResource NkButton}}"
                   Height="28" FontSize="12" Padding="8,0"{foreground}/>"#
    );
    let button: Button = load_xaml(&markup)?.cast()?;
    let content = IReference::from(text).cast::<windows_core::IInspectable>()?;
    button.cast::<ContentControl>()?.SetContent(&content)?;
    AutomationProperties::SetName(&button, name)?;
    Ok(button)
}

/// The plus that adds a binding.
fn add_button(name: &str) -> Result<Button> {
    let button: Button = load_xaml(
        r#"<Button xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation" Style="{StaticResource NkIconButton}"
                   Width="28" Height="28"><FontIcon Glyph="&#xE710;" FontSize="11"/></Button>"#,
    )?
    .cast()?;
    AutomationProperties::SetName(&button, name)?;
    ToolTipService::SetToolTip(&button, &IReference::from(t(Text::KeyAdd)))?;
    Ok(button)
}

impl KeysPage {
    pub fn build(shell: &Rc<Shell>) -> KeysPage {
        let session = Rc::new(Session { active: RefCell::new(None), shell: Rc::downgrade(shell) });
        let mut page = KeysPage { session: Some(session.clone()), ..KeysPage::default() };
        match RestoreRow::new(shell) {
            Ok(row) => page.restore = Some(row),
            Err(e) => log(&format!("drawer restore row: {e}")),
        }
        for command in settings_ui::key_rows() {
            match KeyRow::new(command, shell, &session) {
                Ok(row) => page.rows.push(row),
                Err(e) => log(&format!("drawer key row {}: {e}", command.id())),
            }
        }
        page
    }

    /// The rows of the page in order: the restore button first.
    pub fn elements(&self) -> Vec<FrameworkElement> {
        self.restore.iter().map(|row| row.root.clone()).chain(self.rows.iter().map(|row| row.root.clone())).collect()
    }

    pub fn row(&self, command: Command) -> Option<FrameworkElement> {
        self.rows.iter().find(|row| row.command == command).map(|row| row.root.clone())
    }

    /// Lets go of a chord that is being waited for; true when there was one.
    pub fn stop_recording(&self) -> bool {
        let Some(session) = &self.session else { return false };
        let active = session.active.borrow_mut().take();
        match active {
            Some(row) => {
                row.cancel_capture();
                true
            }
            None => false,
        }
    }

    /// A row that waits for a chord loses the wait when the focus is no longer in it.
    pub fn tick(&self) {
        let Some(session) = &self.session else { return };
        let waiting = session.active.borrow().clone();
        if let Some(row) = waiting
            && !row.inside.get()
        {
            row.cancel_capture();
        }
    }

    /// Brings the rows in line with the keymap and the button that restores them with the settings.
    pub fn sync(&self, shell: &Rc<Shell>) {
        for row in &self.rows {
            row.sync(shell);
        }
        if let Some(restore) = &self.restore {
            let _ = restore
                .button
                .cast::<Control>()
                .and_then(|button| button.SetIsEnabled(!shell.settings.borrow().keybindings.is_empty()));
        }
    }
}

impl RestoreRow {
    fn new(shell: &Rc<Shell>) -> Result<RestoreRow> {
        let (root, slot, _, reset) = row_frame("", false)?;
        ui(&reset)?.SetVisibility(Visibility::Collapsed)?;
        let button: Button = load_xaml(
            r#"<Button xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation" Style="{StaticResource NkButton}"
                       IsEnabled="False"/>"#,
        )?
        .cast()?;
        let content = IReference::from(t(Text::CommonRestoreDefault)).cast::<windows_core::IInspectable>()?;
        button.cast::<ContentControl>()?.SetContent(&content)?;
        slot.SetChild(&button)?;
        let weak = Rc::downgrade(shell);
        let revoker = button.on_click(cb!("drawer restore keys", |_, _| {
            let Some(shell) = weak.upgrade() else { return };
            let again = shell.clone();
            shell.confirm(t(Text::DialogRestoreKeys).to_owned(), Text::CommonRestore, move || {
                again.change_keybindings(|overrides, _| overrides.clear());
            });
        }))?;
        Ok(RestoreRow { root, button, _revoker: revoker })
    }
}

impl KeyRow {
    fn new(command: Command, shell: &Rc<Shell>, session: &Rc<Session>) -> Result<Rc<KeyRow>> {
        let label = command.label(shell.lang());
        let (root, slot, bar, reset) = row_frame(&label, false)?;
        AutomationProperties::SetName(&reset, &tf(Text::LabelValue, &[&t(Text::CommonRestoreDefault), &label]))?;
        ToolTipService::SetToolTip(&reset, &IReference::from(t(Text::CommonRestoreDefault)))?;
        let chips = stack(true, 4.0)?;
        let conflict = stack(true, 8.0)?;
        ui(&conflict)?.SetVisibility(Visibility::Collapsed)?;
        let conflict_text: TextBlock = load_xaml(
            r#"<TextBlock xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation" FontSize="12"
                          VerticalAlignment="Center" Foreground="{StaticResource NkPanelWarning}"
                          TextTrimming="CharacterEllipsis" MaxWidth="170"/>"#,
        )?
        .cast()?;
        let replace = chord_button(t(Text::KeyReplace), &tf(Text::LabelValue, &[&label, &t(Text::KeyReplace)]), false)?;
        let cancel =
            chord_button(t(Text::CommonCancel), &tf(Text::LabelValue, &[&label, &t(Text::CommonCancel)]), false)?;
        for element in [ui(&conflict_text)?, ui(&replace)?, ui(&cancel)?] {
            conflict.cast::<Panel>()?.Children()?.Append(&element)?;
        }
        let holder = stack(true, 0.0)?;
        for element in [ui(&chips)?, ui(&conflict)?] {
            holder.cast::<Panel>()?.Children()?.Append(&element)?;
        }
        slot.SetChild(&holder)?;

        let row = Rc::new(KeyRow {
            command,
            root: root.clone(),
            bar,
            reset,
            chips,
            conflict,
            conflict_text,
            replace,
            cancel,
            capture: Cell::new(Capture::Idle),
            recorder: RefCell::new(None),
            drawing: Cell::new(false),
            inside: Cell::new(false),
            shown: RefCell::new((Vec::new(), false)),
            session: session.clone(),
            chip_revokers: RefCell::new(Vec::new()),
            _revokers: RefCell::new(Vec::new()),
        });
        let mut revokers = Vec::new();
        let this = row.clone();
        revokers.push(row.reset.on_click(cb!("drawer key reset", |_, _| {
            if let Some(shell) = this.session.shell.upgrade() {
                this.cancel_capture();
                shell.change_keybindings(|overrides, _| keybindings::reset(overrides, command));
            }
        }))?);
        let this = row.clone();
        revokers.push(row.replace.on_click(cb!("drawer key replace", |_, _| {
            if let Capture::Conflict { replacing, chord, .. } = this.capture.get() {
                this.give(replacing, chord);
            }
        }))?);
        let this = row.clone();
        revokers.push(row.cancel.on_click(cb!("drawer key cancel", |_, _| this.cancel_capture()))?);
        let this = row.clone();
        revokers.push(ui(&root)?.PreviewKeyDown(cb!("drawer key capture", |_, args| {
            let Some(args) = args.as_ref() else { return };
            this.on_key(args);
        }))?);
        // The focus goes elsewhere (another section, the terminal): nothing is waiting for a chord any more. The frame
        // after, `KeysPage::tick` sees it; moving between the buttons of the row is a loss and a gain in one frame.
        let this = row.clone();
        revokers.push(ui(&root)?.GotFocus(cb!("drawer key focus", |_, _| this.inside.set(true)))?);
        let this = row.clone();
        revokers.push(ui(&root)?.LostFocus(cb!("drawer key blur", |_, _| {
            if !this.drawing.get() {
                this.inside.set(false);
            }
        }))?);
        *row._revokers.borrow_mut() = revokers;
        row.draw(shell);
        Ok(row)
    }

    /// Redraws the buttons of the row for what it is doing: its shortcuts, or what to say about the chord that came.
    fn draw(self: &Rc<Self>, shell: &Rc<Shell>) {
        self.drawing.set(true);
        if let Err(e) = self.draw_buttons(shell) {
            log(&format!("drawer key row {}: {e}", self.command.id()));
        }
        self.drawing.set(false);
    }

    fn draw_buttons(self: &Rc<Self>, shell: &Rc<Shell>) -> Result<()> {
        let lang = shell.lang();
        let label = self.command.label(lang);
        let shortcuts: Vec<Shortcut> = keybindings::current(&shell.keymap.borrow(), self.command);
        let capture = self.capture.get();
        let children = self.chips.cast::<Panel>()?.Children()?;
        children.Clear()?;
        self.chip_revokers.borrow_mut().clear();
        *self.recorder.borrow_mut() = None;
        let conflicting = matches!(capture, Capture::Conflict { .. });
        ui(&self.chips)?.SetVisibility(visibility(!conflicting))?;
        ui(&self.conflict)?.SetVisibility(visibility(conflicting))?;
        if let Capture::Conflict { clash, .. } = capture {
            let sentence = match clash {
                Clash::Command(owner) => tf(Text::KeyConflict, &[&owner.label(lang)]),
                Clash::Program => t(Text::KeyReserved).to_owned(),
            };
            self.conflict_text.SetText(&sentence)?;
            ToolTipService::SetToolTip(&self.conflict_text, &IReference::from(sentence.as_str()))?;
        } else {
            let waiting_for = match capture {
                Capture::Recording { replacing } => Some(replacing),
                _ => None,
            };
            for shortcut in &shortcuts {
                let recording = waiting_for == Some(Some(*shortcut));
                // The accessible name says what the button shows: the chord, or that a key is waited for.
                let text = if recording { t(Text::KeyRecord).to_owned() } else { shortcut.to_string() };
                let button = chord_button(&text, &tf(Text::LabelValue, &[&label, &text]), recording)?;
                let (this, chord) = (self.clone(), *shortcut);
                let click = button.on_click(cb!("drawer key chip", |_, _| this.begin(Some(chord))))?;
                self.chip_revokers.borrow_mut().push(click);
                children.Append(&button)?;
                if recording {
                    *self.recorder.borrow_mut() = Some(button);
                }
            }
            if waiting_for == Some(None) {
                let button =
                    chord_button(t(Text::KeyRecord), &tf(Text::LabelValue, &[&label, &t(Text::KeyRecord)]), true)?;
                children.Append(&button)?;
                *self.recorder.borrow_mut() = Some(button);
            } else {
                let button = add_button(&tf(Text::LabelValue, &[&label, &t(Text::KeyAdd)]))?;
                let this = self.clone();
                let click = button.on_click(cb!("drawer key add", |_, _| this.begin(None)))?;
                self.chip_revokers.borrow_mut().push(click);
                children.Append(&button)?;
            }
        }
        let modified = keybindings::is_modified(&shell.settings.borrow().keybindings, self.command);
        ui(&self.bar)?.SetOpacity(if modified { 1.0 } else { 0.0 })?;
        ui(&self.reset)?.SetOpacity(if modified { 1.0 } else { 0.0 })?;
        self.reset.cast::<Control>()?.SetIsEnabled(modified)?;
        *self.shown.borrow_mut() = (shortcuts.iter().map(ToString::to_string).collect(), modified);
        Ok(())
    }

    /// Brings the row in line with the keymap, unless it is waiting for a chord (the buttons must stay as they are).
    fn sync(self: &Rc<Self>, shell: &Rc<Shell>) {
        if self.capture.get() != Capture::Idle {
            return;
        }
        let now: Vec<String> =
            keybindings::current(&shell.keymap.borrow(), self.command).iter().map(ToString::to_string).collect();
        let modified = keybindings::is_modified(&shell.settings.borrow().keybindings, self.command);
        if *self.shown.borrow() != (now, modified) {
            self.draw(shell);
        }
    }

    /// Starts waiting for a chord, to replace `replacing` or as one more binding.
    fn begin(self: &Rc<Self>, replacing: Option<Shortcut>) {
        let Some(shell) = self.session.shell.upgrade() else { return };
        let previous = self.session.active.replace(Some(self.clone()));
        if let Some(previous) = previous
            && !Rc::ptr_eq(&previous, self)
        {
            previous.cancel_capture();
        }
        shell.recording_keys.set(true);
        self.capture.set(Capture::Recording { replacing });
        self.draw(&shell);
        self.focus_recorder();
        self.inside.set(true);
    }

    fn focus_recorder(&self) {
        let recorder = self.recorder.borrow().clone();
        if let Some(button) = recorder {
            let _ = ui(&button).and_then(|button| button.Focus(FocusState::Keyboard));
        }
    }

    /// Stops waiting; the row shows its shortcuts again and the keyboard layer runs commands again.
    fn cancel_capture(self: &Rc<Self>) {
        let Some(shell) = self.session.shell.upgrade() else { return };
        let was = self.capture.replace(Capture::Idle);
        shell.recording_keys.set(false);
        {
            let mut active = self.session.active.borrow_mut();
            if active.as_ref().is_some_and(|row| Rc::ptr_eq(row, self)) {
                *active = None;
            }
        }
        if was != Capture::Idle {
            self.draw(&shell);
            self.focus_first();
        }
    }

    /// The focus goes to the first button of the row: where the user was.
    fn focus_first(&self) {
        let first =
            self.chips.cast::<Panel>().and_then(|panel| panel.Children()).and_then(|children| children.GetAt(0));
        let _ = first.and_then(|button| button.Focus(FocusState::Keyboard));
    }

    /// Gives the chord to the command and goes back to showing the shortcuts.
    fn give(self: &Rc<Self>, replacing: Option<Shortcut>, chord: Shortcut) {
        let Some(shell) = self.session.shell.upgrade() else { return };
        let command = self.command;
        self.capture.set(Capture::Idle);
        shell.recording_keys.set(false);
        *self.session.active.borrow_mut() = None;
        shell.change_keybindings(|overrides, keymap| keybindings::assign(overrides, keymap, command, replacing, chord));
        self.draw(&shell);
        self.focus_first();
    }

    /// A key while the row waits: a chord is given to the command at once unless something has it; Backspace and Delete
    /// take the binding that is being replaced away.
    fn on_key(self: &Rc<Self>, args: &KeyRoutedEventArgs) {
        let Capture::Recording { replacing } = self.capture.get() else { return };
        let code = args.Key().map_or(0, |key| key.0);
        let _ = args.SetHandled(true);
        // Esc is the drawer's (it lets go of the chord); a modifier on its own is not a chord yet.
        if code == VK_ESCAPE || keyboard::is_modifier_key(code) {
            return;
        }
        let held = keyboard::held_modifiers();
        if held.meta {
            return;
        }
        let none_held = !(held.ctrl || held.alt || held.shift);
        if none_held && matches!(code, VK_BACK | VK_DELETE) {
            if let (Some(old), Some(shell)) = (replacing, self.session.shell.upgrade()) {
                let command = self.command;
                self.capture.set(Capture::Idle);
                shell.recording_keys.set(false);
                *self.session.active.borrow_mut() = None;
                shell.change_keybindings(|overrides, keymap| keybindings::remove(overrides, keymap, command, old));
                self.draw(&shell);
                self.focus_first();
            }
            return;
        }
        let status = args.KeyStatus().unwrap_or_default();
        let Some(chord) = keyboard::physical_code(code, &status).and_then(|key| keybindings::chord_of(key, held))
        else {
            return;
        };
        let Some(shell) = self.session.shell.upgrade() else { return };
        // The keymap is borrowed only for the question: giving the chord changes it.
        let found = keybindings::clash(&shell.keymap.borrow(), chord, self.command);
        match found {
            None => self.give(replacing, chord),
            Some(clash) => {
                self.capture.set(Capture::Conflict { replacing, chord, clash });
                self.draw(&shell);
                let _ = ui(&self.cancel).and_then(|cancel| cancel.Focus(FocusState::Keyboard));
            }
        }
    }
}
