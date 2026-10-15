//! The pages of the settings drawer that are not rows of `settings_ui::TABLE`: the shortcuts (one row per command of the
//! palette), and the advanced page, which says what the application is (version, ConPTY, the settings file) and lists
//! what is wrong with that file.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use nekominal_model::commands::Command;
use nekominal_model::settings::Problem;
use nekominal_model::settings_ui::Section;
use nekominal_model::strings::Text;
use nekominal_winui_sys::*;
use windows_core::{EventRevoker, Interface, Result};
use windows_reference::IReference;

use crate::drawer_keys::KeysPage;
use crate::drawer_rows::{row_frame, stack};
use crate::i18n::{t, tf};
use crate::shell::Shell;
use crate::util::*;

/// The shortcuts and the advanced page.
#[derive(Default)]
pub struct Extras {
    keys: KeysPage,
    advanced: Vec<FrameworkElement>,
    problems: Option<ProblemList>,
}

/// What is wrong with the settings file: a heading with the action that opens the file, and a row per problem. It is
/// there only while the file has problems.
struct ProblemList {
    root: StackPanel,
    shown: RefCell<Vec<Problem>>,
    shell: Weak<Shell>,
    /// The handler of the button of the heading, which goes when the heading does.
    revokers: RefCell<Vec<EventRevoker>>,
}

impl Extras {
    /// Builds the pages that list things.
    pub fn build(shell: &Rc<Shell>) -> Extras {
        let mut extras = Extras::default();
        extras.keys = KeysPage::build(shell);
        let mut about: Vec<(&str, String, bool)> =
            vec![(t(Text::SettingsVersion), env!("CARGO_PKG_VERSION").to_owned(), false)];
        if let Ok(backend) = nekominal_pty::conpty::backend() {
            let conpty = if backend.kind == nekominal_pty::conpty::Backend::BUNDLED {
                tf(Text::ConptyBundled, &[&backend.version])
            } else {
                t(Text::ConptySystem).to_owned()
            };
            about.push((t(Text::SettingsConpty), conpty, false));
        }
        if let Some(path) = nekominal_model::store::settings_path() {
            about.push((t(Text::SettingsFile), path.display().to_string(), true));
        }
        for (label, value, wide) in about {
            match info_row(label, &value, wide) {
                Ok(row) => extras.advanced.push(row),
                Err(e) => log(&format!("drawer info row {label}: {e}")),
            }
        }
        match stack(false, 0.0) {
            Ok(root) => {
                if let Ok(element) = fe(&root) {
                    extras.advanced.push(element);
                }
                extras.problems = Some(ProblemList {
                    root,
                    shown: RefCell::new(Vec::new()),
                    shell: Rc::downgrade(shell),
                    revokers: RefCell::new(Vec::new()),
                });
            }
            Err(e) => log(&format!("drawer problems: {e}")),
        }
        extras
    }

    pub fn key_row(&self, command: Command) -> Option<FrameworkElement> {
        self.keys.row(command)
    }

    /// The rows a section has besides the ones of the table.
    pub fn section_rows(&self, section: Section) -> Vec<FrameworkElement> {
        match section {
            Section::Keys => self.keys.elements(),
            Section::Advanced => self.advanced.clone(),
            _ => Vec::new(),
        }
    }

    /// Lets go of a shortcut that is being recorded; true when there was one.
    pub fn stop_recording(&self) -> bool {
        self.keys.stop_recording()
    }

    /// Once per frame while the drawer is open.
    pub fn tick(&self) {
        self.keys.tick();
    }

    /// Brings the list of problems in line with the settings file.
    pub fn sync(&self, shell: &Rc<Shell>) {
        self.keys.sync(shell);
        let Some(list) = &self.problems else { return };
        if *list.shown.borrow() != *shell.problems.borrow() {
            if let Err(e) = list.rebuild(shell) {
                log(&format!("drawer problems: {e}"));
            }
        }
    }
}

impl ProblemList {
    /// A heading (the word, and the action that opens the file) and one row per problem; nothing at all without any.
    fn rebuild(&self, shell: &Rc<Shell>) -> Result<()> {
        let problems = shell.problems.borrow().clone();
        let children = self.root.cast::<Panel>()?.Children()?;
        children.Clear()?;
        self.revokers.borrow_mut().clear();
        ui(&self.root)?.SetVisibility(visibility(!problems.is_empty()))?;
        if !problems.is_empty() {
            let (heading, slot, _, reset) = row_frame(t(Text::SettingsProblems), false)?;
            ui(&reset)?.SetVisibility(Visibility::Collapsed)?;
            let open: Button = load_xaml(
                r#"<Button xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation" Style="{StaticResource NkButton}"/>"#,
            )?
            .cast()?;
            let label = IReference::from(t(Text::AppOpenSettingsFile)).cast::<windows_core::IInspectable>()?;
            open.cast::<ContentControl>()?.SetContent(&label)?;
            let owner = self.shell.clone();
            let click = open.on_click(cb!("drawer open settings file", |_, _| {
                if let Some(shell) = owner.upgrade() {
                    shell.run(Command::AppOpenSettingsFile);
                }
            }))?;
            self.revokers.borrow_mut().push(click);
            slot.SetChild(&open)?;
            children.Append(&ui(&heading)?)?;
            let lang = shell.lang();
            for problem in &problems {
                // A problem with the whole file has no key: it is about the file.
                let label = if problem.key.is_empty() { t(Text::SettingsFile) } else { &problem.key };
                children.Append(&ui(&info_row(label, &problem.expected.text(lang), true)?)?)?;
            }
        }
        *self.shown.borrow_mut() = problems;
        Ok(())
    }
}

/// A row that only says something: a label and a text. A long text (a path, what a problem is) goes under the label.
fn info_row(label: &str, value: &str, wide: bool) -> Result<FrameworkElement> {
    let (root, slot, _, reset) = row_frame(label, wide)?;
    let text: TextBlock = load_xaml(
        r#"<TextBlock xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation" FontSize="13"
                      Foreground="{StaticResource NkPanelTextSecondary}" IsTextSelectionEnabled="True"
                      TextTrimming="CharacterEllipsis"/>"#,
    )?
    .cast()?;
    if wide {
        text.SetTextWrapping(TextWrapping::Wrap)?;
        text.SetTextTrimming(TextTrimming::None)?;
    }
    text.SetText(value)?;
    slot.SetChild(&text)?;
    ui(&reset)?.SetVisibility(Visibility::Collapsed)?;
    Ok(root)
}
