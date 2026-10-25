//! Opening a checked file at a line in the user's editor (the setting `editor`, see `nekominal_model::editor`), and
//! everything else with its default program as before.
//!
//! The callers hand over what `nekominal_term::links::openable` let through: a URL, or the full path of a file that is
//! there and does not run code. The editor is started directly with its arguments, never through a shell, and only
//! when the text gave a line to go to.

use std::path::PathBuf;
use std::process::Command;

use nekominal_model::editor;

use crate::util::log;

/// Opens `target` at `position` (the line and column the text named, see `links::position`) in the editor the setting
/// names; with no position, no editor, or an editor that does not start, with its default program.
pub fn open(target: &str, position: Option<(u32, Option<u32>)>, setting: &str) {
    // A URL is never a path, so it never reaches an editor.
    let file = std::path::Path::new(target).is_absolute();
    if let Some((line, column)) = position.filter(|_| file)
        && let Some(template) = template(setting)
    {
        match launch(&template, target, line, column.unwrap_or(1)) {
            Ok(()) => return,
            Err(e) => log(&format!("editor: {e}")),
        }
    }
    if let Err(e) = crate::open::shell_open(target) {
        log(&format!("open link: {e}"));
    }
}

/// The command line of the setting: the first editor found for `auto`, none when the setting is empty.
fn template(setting: &str) -> Option<String> {
    match setting.trim() {
        "" => None,
        editor::AUTO => {
            let found: Vec<&str> = editor::KNOWN.iter().map(|(name, _)| *name).filter(|name| locate(name).is_some()).collect();
            editor::pick(&found).map(str::to_owned)
        }
        command => Some(command.to_owned()),
    }
}

/// Where a program is: a path as written, a name on PATH (a `.cmd` too, which is how VS Code and its kin put themselves
/// there), or the folder the installer of one of the known editors uses when it is not on PATH.
fn locate(program: &str) -> Option<PathBuf> {
    let found = nekominal_pty::profiles::resolve_program(program)
        .or_else(|| (std::path::Path::new(program).extension().is_none()).then(|| nekominal_pty::profiles::resolve_program(&format!("{program}.cmd"))).flatten());
    found.or_else(|| {
        let installed: &[&str] = match program {
            "code" => &[r"%LOCALAPPDATA%\Programs\Microsoft VS Code\bin\code.cmd", r"%ProgramFiles%\Microsoft VS Code\bin\code.cmd"],
            "cursor" => &[r"%LOCALAPPDATA%\Programs\cursor\resources\app\bin\cursor.cmd"],
            "windsurf" => &[r"%LOCALAPPDATA%\Programs\Windsurf\bin\windsurf.cmd"],
            "zed" => &[r"%LOCALAPPDATA%\Programs\Zed\bin\zed.exe"],
            _ => &[],
        };
        installed.iter().filter_map(|path| expand(path)).find(|path| path.is_file())
    })
}

/// `%NAME%\rest` with the variable put in; `None` when it is not set.
fn expand(path: &str) -> Option<PathBuf> {
    let (variable, rest) = path.strip_prefix('%')?.split_once('%')?;
    Some(PathBuf::from(std::env::var_os(variable)?).join(rest.trim_start_matches('\\')))
}

fn launch(template: &str, file: &str, line: u32, column: u32) -> Result<(), String> {
    let (program, args) = editor::command(template, file, line, column).ok_or_else(|| format!("not a command with {{file}}: {template}"))?;
    let path = locate(&program).ok_or_else(|| format!("{program} was not found"))?;
    if std::env::var_os(crate::open::NO_LAUNCH).is_some() {
        log(&format!("not started ({} is set): {} {args:?}", crate::open::NO_LAUNCH, path.display()));
        return Ok(());
    }
    use std::os::windows::process::CommandExt;
    // A `.cmd` runs in cmd.exe: without this a console window flashes up. The standard library quotes each argument for
    // it, and refuses one it cannot pass safely.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    Command::new(&path).args(&args).creation_flags(CREATE_NO_WINDOW).spawn().map(drop).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_install_folder_is_read_from_its_variable() {
        assert_eq!(expand(r"%SystemRoot%\System32\cmd.exe"), std::env::var_os("SystemRoot").map(|root| PathBuf::from(root).join(r"System32\cmd.exe")));
        assert_eq!(expand(r"%NEKOMINAL_NO_SUCH_VARIABLE%\x"), None);
        assert_eq!(expand(r"C:\x"), None);
    }

    #[test]
    fn an_empty_setting_uses_the_default_program_and_a_command_is_used_as_written() {
        assert_eq!(template(""), None);
        assert_eq!(template(" notepad {file} ").as_deref(), Some("notepad {file}"));
    }
}
