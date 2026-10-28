//! Copies of the app's programs outside its folder. Scoop refuses to update an app while any process runs from its folder
//! (`test_running_process` of its `install.ps1`), so what keeps running across an update (the OpenConsole of every pane,
//! the copy of Nekominal that holds the tabs) runs from `%LOCALAPPDATA%\Nekominal\<kind>\<first 12 hex digits of the SHA-256 of the files>` instead.
//! A folder is named by what it holds: the same files are copied once, and a new version gets a folder of its own while
//! the old one is still in use. The folders no running program uses are deleted by the next start.

use std::{
    fs, io,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use windows_sys::Win32::Security::Cryptography::{BCRYPT_SHA256_ALG_HANDLE, BCryptHash};

/// A folder of another hash younger than this is left alone: another start may have just made it and not yet loaded it.
const KEEP_YOUNG: Duration = Duration::from_secs(10 * 60);

/// The SHA-256 of `data`.
pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut digest = [0u8; 32];
    // SAFETY: the pseudo handle of the SHA-256 provider needs no opening; the buffers are as long as the lengths given.
    let status = unsafe { BCryptHash(BCRYPT_SHA256_ALG_HANDLE, std::ptr::null(), 0, data.as_ptr(), data.len() as u32, digest.as_mut_ptr(), 32) };
    assert!(status >= 0, "BCryptHash failed with {status:#x}");
    digest
}

/// The name of the folder of `files`' contents: the first 12 hex digits of the SHA-256 of them in order.
fn folder_name(contents: &[Vec<u8>]) -> String {
    let digest = sha256(&contents.concat());
    digest[..6].iter().map(|byte| format!("{byte:02x}")).collect()
}

/// `files` (those that exist; the first must) copied into `%LOCALAPPDATA%\Nekominal\<kind>\<hash>`, and that folder.
pub fn relocate(kind: &str, files: &[PathBuf]) -> io::Result<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA").ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "LOCALAPPDATA is not set"))?;
    relocate_in(&PathBuf::from(base).join("Nekominal").join(kind), files)
}

/// `file` copied as `name` into `%LOCALAPPDATA%\Nekominal\<kind>\<hash>`, and that folder: a program that runs from there
/// under another name (Nekominal itself, as the copy that holds the tabs through an update).
pub fn relocate_as(kind: &str, file: &Path, name: &str) -> io::Result<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA").ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "LOCALAPPDATA is not set"))?;
    relocate_in_as(&PathBuf::from(base).join("Nekominal").join(kind), file, name)
}

/// [`relocate_as`] into `root`.
pub fn relocate_in_as(root: &Path, file: &Path, name: &str) -> io::Result<PathBuf> {
    copy_into(root, &[file.to_path_buf()], Some(name))
}

/// [`relocate`] into `root`. The first file is the one a program that uses the folder holds open (the DLL it loaded, the
/// program it runs): a folder of another hash is deleted only when that file can be, so nothing in use loses a file.
pub fn relocate_in(root: &Path, files: &[PathBuf]) -> io::Result<PathBuf> {
    copy_into(root, files, None)
}

/// The copying of [`relocate_in`], the first file named `rename` when given.
fn copy_into(root: &Path, files: &[PathBuf], rename: Option<&str>) -> io::Result<PathBuf> {
    let first = files.first().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no file to copy"))?;
    let present: Vec<&PathBuf> = files.iter().filter(|file| file.is_file()).collect();
    if present.first() != Some(&first) {
        return Err(io::Error::new(io::ErrorKind::NotFound, format!("{} is missing", first.display())));
    }
    let contents = present.iter().map(fs::read).collect::<io::Result<Vec<_>>>()?;
    let name = folder_name(&contents);
    let dir = root.join(&name);
    fs::create_dir_all(&dir)?;
    let held = rename.map(std::ffi::OsStr::new).or(first.file_name()).unwrap_or_default().to_owned();
    for (index, (file, content)) in present.iter().zip(&contents).enumerate() {
        let file_name = if index == 0 { Some(held.as_os_str()) } else { file.file_name() };
        let target = dir.join(file_name.ok_or_else(|| io::Error::other("a file without a name"))?);
        if fs::read(&target).is_ok_and(|there| there == *content) {
            continue;
        }
        // Beside and moved over: two starts at once never see half a file.
        let part = target.with_extension(format!("{}.part", std::process::id()));
        fs::write(&part, content)?;
        if let Err(e) = fs::rename(&part, &target) {
            let _ = fs::remove_file(&part);
            // The other start won: its copy is the same.
            if !fs::read(&target).is_ok_and(|there| there == *content) {
                return Err(e);
            }
        }
    }
    clean(root, &name, &held);
    Ok(dir)
}

/// Deletes the folders of `root` other than `keep` that no program uses: their `held` file can be deleted.
fn clean(root: &Path, keep: &str, held: &std::ffi::OsStr) {
    let Ok(entries) = fs::read_dir(root) else { return };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let path = entry.path();
        let young = entry.metadata().and_then(|m| m.modified()).is_ok_and(|at| now.duration_since(at).is_ok_and(|age| age < KEEP_YOUNG) || at > now);
        if entry.file_name() == keep || !path.is_dir() || young {
            continue;
        }
        let lock = path.join(held);
        if !lock.exists() || fs::remove_file(&lock).is_ok() {
            let _ = fs::remove_dir_all(&path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_is_the_standard_digest() {
        let hex: String = sha256(b"abc").iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn files_are_copied_once_into_a_folder_named_by_their_contents_and_unused_old_folders_go() {
        let work = std::env::temp_dir().join(format!("nekominal-relocate-{}", std::process::id()));
        let _ = fs::remove_dir_all(&work);
        let (source, root) = (work.join("app"), work.join("copies"));
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("a.dll"), b"one").unwrap();
        fs::write(source.join("b.exe"), b"two").unwrap();
        let files = [source.join("a.dll"), source.join("b.exe"), source.join("VERSION")];

        let dir = relocate_in(&root, &files).unwrap();
        assert_eq!(dir.file_name().unwrap().len(), 12);
        assert_eq!(fs::read(dir.join("b.exe")).unwrap(), b"two");
        assert!(!dir.join("VERSION").exists(), "a file that is not there is left out");
        assert_eq!(relocate_in(&root, &files).unwrap(), dir, "the same files, the same folder");

        // An old folder in use (its first file held open) stays; one nobody uses goes, unless it was just made.
        let old = |name: &str| {
            let at = root.join(name);
            fs::create_dir_all(&at).unwrap();
            fs::write(at.join("a.dll"), b"old").unwrap();
            // FILE_FLAG_BACKUP_SEMANTICS: a folder opens only with it.
            use std::os::windows::fs::OpenOptionsExt;
            let folder = fs::File::options().write(true).custom_flags(0x0200_0000).open(&at).unwrap();
            folder.set_modified(SystemTime::now() - Duration::from_secs(3600)).unwrap();
            at
        };
        let (used, unused, young) = (old("000000000001"), old("000000000002"), root.join("000000000003"));
        fs::create_dir_all(&young).unwrap();
        // Held without FILE_SHARE_DELETE, as a loaded DLL or a running program is.
        let held = {
            use std::os::windows::fs::OpenOptionsExt;
            fs::File::options().read(true).share_mode(1).open(used.join("a.dll")).unwrap()
        };
        fs::write(source.join("b.exe"), b"three").unwrap();
        let newer = relocate_in(&root, &files).unwrap();
        assert_ne!(newer, dir, "new contents, a new folder");
        assert!(used.join("a.dll").exists(), "a folder in use keeps its files");
        assert!(!unused.exists(), "an unused folder goes");
        assert!(young.exists(), "a folder just made stays");
        drop(held);
        let _ = fs::remove_dir_all(&work);
    }

    #[test]
    fn the_first_file_must_be_there() {
        let missing = std::env::temp_dir().join("nekominal-relocate-missing.dll");
        assert!(relocate_in(&std::env::temp_dir(), &[missing]).is_err());
    }
}
