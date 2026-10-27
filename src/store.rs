//! On-disk storage: every prompt is one `.aiprompt` file in `~/.wordesign`.
//!
//! Set `WORDESIGN_DIR` to use a different folder (handy for testing).

use crate::model::Prompt;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub const EXT: &str = "aiprompt";

#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub modified: SystemTime,
    /// Sub-folder of the library this prompt lives in (`None` = top level).
    pub folder: Option<String>,
}

fn home_dir() -> Option<PathBuf> {
    let get = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    get("HOME")
        .or_else(|| get("USERPROFILE"))
        .or_else(|| match (get("HOMEDRIVE"), get("HOMEPATH")) {
            (Some(d), Some(p)) => Some(PathBuf::from(format!("{}{}", d.display(), p.display()))),
            _ => None,
        })
}

/// `~/.wordesign` (or `$WORDESIGN_DIR`), created if missing.
pub fn base_dir() -> io::Result<PathBuf> {
    let dir = match std::env::var_os("WORDESIGN_DIR").filter(|v| !v.is_empty()) {
        Some(d) => PathBuf::from(d),
        None => home_dir()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "cannot determine home directory"))?
            .join(".wordesign"),
    };
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// All prompts in `dir`, most recently modified first.
pub fn list(dir: &Path) -> Vec<Entry> {
    let mut out: Vec<Entry> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            if path.extension().and_then(|x| x.to_str()) != Some(EXT) {
                return None;
            }
            let meta = e.metadata().ok()?;
            if !meta.is_file() {
                return None;
            }
            let name = path.file_stem()?.to_string_lossy().to_string();
            Some(Entry {
                name,
                path,
                modified: meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                folder: None,
            })
        })
        .collect();
    out.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| a.name.cmp(&b.name)));
    out
}

/// Turn user input into a safe file stem. Returns "" if nothing usable is left.
pub fn sanitize(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let mut s = cleaned.trim().trim_matches('.').trim().to_string();
    // At most 80 characters AND 180 bytes: CJK characters take 3 bytes and emoji 4,
    // while file names are limited to 255 bytes.
    while s.chars().count() > 80 || s.len() > 180 {
        s.pop();
    }
    s = s.trim_end().trim_end_matches('.').trim_end().to_string();
    // Reserved device names on Windows.
    let upper = s.to_ascii_uppercase();
    let reserved = matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (upper.len() == 4
            && (upper.starts_with("COM") || upper.starts_with("LPT"))
            && upper.as_bytes()[3].is_ascii_digit());
    if reserved {
        s.push('_');
    }
    s
}

pub fn path_for(dir: &Path, stem: &str) -> PathBuf {
    dir.join(format!("{stem}.{EXT}"))
}

/// `dir/stem.aiprompt`, or `stem_2`, `stem_3`, ... if taken (ignoring `ignore`,
/// which is the file being renamed itself).
pub fn unique_path(dir: &Path, stem: &str, ignore: Option<&Path>) -> PathBuf {
    let free = |p: &Path| !p.exists() || ignore == Some(p);
    let first = path_for(dir, stem);
    if free(&first) {
        return first;
    }
    (2..)
        .map(|n| path_for(dir, &format!("{stem}_{n}")))
        .find(|p| free(p))
        .expect("infinite iterator")
}

/// Lowest `prompt_N` (N >= 1) that is neither on disk nor in `taken`.
pub fn default_stem(dir: &Path, taken: &[&str]) -> String {
    (1..)
        .map(|n| format!("prompt_{n}"))
        .find(|s| !path_for(dir, s).exists() && !taken.contains(&s.as_str()))
        .expect("infinite iterator")
}

// ---------------------------------------------------------------------
// Folders: real sub-directories of the library, one level deep.
// ---------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct Library {
    /// Folder names, alphabetical (empty folders included).
    pub folders: Vec<String>,
    /// Every prompt in the library (top level and folders), most recently modified first.
    pub entries: Vec<Entry>,
}

/// Read the whole library: top-level prompts plus one level of folders.
/// The built-in folder whose prompts expire automatically.
pub const TEMP: &str = "temp";

pub fn scan(base: &Path) -> Library {
    let mut entries = list(base);
    let mut folders: Vec<String> = Vec::new();
    for e in std::fs::read_dir(base).into_iter().flatten().flatten() {
        let path = e.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()).map(str::to_owned) else {
            continue;
        };
        if name.starts_with('.') {
            continue;
        }
        for mut entry in list(&path) {
            entry.folder = Some(name.clone());
            entries.push(entry);
        }
        folders.push(name);
    }
    folders.sort_by_key(|n| n.to_lowercase());
    // The temp folder always exists and is listed last.
    folders.retain(|n| n != TEMP);
    folders.push(TEMP.to_owned());
    entries.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| a.name.cmp(&b.name)));
    Library { folders, entries }
}

/// Delete prompts in the temp folder that were last changed more than `max_age` ago
/// (never `keep`, the prompt that is open). Returns how many were removed.
pub fn purge_temp(base: &Path, max_age: std::time::Duration, keep: Option<&Path>) -> usize {
    let dir = base.join(TEMP);
    let mut removed = 0;
    for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
        let path = e.path();
        if path.extension().and_then(|x| x.to_str()) != Some(EXT) || Some(path.as_path()) == keep {
            continue;
        }
        let age = e
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok());
        if age.is_some_and(|a| a > max_age) && std::fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// Permanently delete a folder and everything in it. The built-in temp folder cannot be deleted.
pub fn delete_folder(base: &Path, name: &str) -> io::Result<()> {
    let bad = name.is_empty()
        || name == TEMP
        || name.starts_with('.')
        || name.contains(['/', '\\'])
        || name.contains("..");
    if bad {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "this folder cannot be deleted"));
    }
    let dir = base.join(name);
    if dir.parent() != Some(base) || !dir.is_dir() {
        return Err(io::Error::new(io::ErrorKind::NotFound, "no such folder"));
    }
    std::fs::remove_dir_all(dir)
}

// ---------------------------------------------------------------------
// reusable contexts: plain text files in the hidden `.contexts` folder
// ---------------------------------------------------------------------

const CONTEXT_DIR: &str = ".contexts";
const CONTEXT_EXT: &str = "txt";

fn context_path(base: &Path, name: &str) -> PathBuf {
    base.join(CONTEXT_DIR).join(format!("{}.{CONTEXT_EXT}", sanitize(name)))
}

/// Names of the saved reusable contexts, alphabetical (case-insensitive).
pub fn list_contexts(base: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(base.join(CONTEXT_DIR))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == CONTEXT_EXT))
        .filter_map(|e| e.path().file_stem().map(|s| s.to_string_lossy().into_owned()))
        .collect();
    v.sort_by_key(|n| n.to_lowercase());
    v
}

pub fn read_context(base: &Path, name: &str) -> io::Result<String> {
    std::fs::read_to_string(context_path(base, name))
}

/// Save (or overwrite) a reusable context; returns the name it was stored under.
pub fn write_context(base: &Path, name: &str, text: &str) -> io::Result<String> {
    let clean = sanitize(name);
    if clean.is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "give the context a name"));
    }
    std::fs::create_dir_all(base.join(CONTEXT_DIR))?;
    write_atomic(&context_path(base, &clean), text)?;
    Ok(clean)
}

pub fn delete_context(base: &Path, name: &str) -> io::Result<()> {
    std::fs::remove_file(context_path(base, name))
}

/// Permanently delete one prompt file (must be a `.aiprompt` inside the library).
pub fn delete_prompt(base: &Path, path: &Path) -> io::Result<()> {
    let inside = path.starts_with(base) && !path.components().any(|c| matches!(c, std::path::Component::ParentDir));
    let is_prompt = path.extension().is_some_and(|e| e == EXT);
    if !inside || !is_prompt || !path.is_file() {
        return Err(io::Error::new(io::ErrorKind::NotFound, "no such prompt"));
    }
    std::fs::remove_file(path)
}

/// Directory that holds prompts of `folder` (`None` = the library root).
pub fn folder_dir(base: &Path, folder: Option<&str>) -> PathBuf {
    match folder {
        Some(f) => base.join(f),
        None => base.to_path_buf(),
    }
}

/// Which folder a prompt file is in.
pub fn folder_of(base: &Path, path: &Path) -> Option<String> {
    let parent = path.parent()?;
    if parent == base {
        return None;
    }
    parent.file_name().and_then(|n| n.to_str()).map(str::to_owned)
}

/// Create a folder (or reuse an existing one whose name matches ignoring case).
/// Returns the folder's real name.
pub fn create_folder(base: &Path, name: &str) -> io::Result<String> {
    let stem = sanitize(name);
    if stem.is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "folder name is empty"));
    }
    std::fs::create_dir_all(base)?;
    for e in std::fs::read_dir(base)?.flatten() {
        if e.path().is_dir() {
            if let Some(n) = e.file_name().to_str() {
                if n.to_lowercase() == stem.to_lowercase() {
                    return Ok(n.to_owned());
                }
            }
        }
    }
    std::fs::create_dir(base.join(&stem))?;
    Ok(stem)
}

/// Where `path` would live inside `folder` (never clobbers another prompt).
pub fn retarget(base: &Path, path: &Path, folder: Option<&str>) -> PathBuf {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    unique_path(&folder_dir(base, folder), &stem, Some(path))
}

/// Move an existing prompt file into `folder`. Returns its new path.
pub fn move_to_folder(base: &Path, path: &Path, folder: Option<&str>) -> io::Result<PathBuf> {
    let target = retarget(base, path, folder);
    if target != path {
        if let Some(dir) = target.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::rename(path, &target)?;
    }
    Ok(target)
}

/// Write via a temp file + rename so a crash can never leave a half-written prompt.
pub fn write_atomic(path: &Path, contents: &str) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension(format!("{EXT}.tmp"));
    std::fs::write(&tmp, contents)?;
    match std::fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

pub fn load(path: &Path) -> io::Result<Prompt> {
    let bytes = std::fs::read(path)?;
    Ok(Prompt::from_file(&String::from_utf8_lossy(&bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::BodyBlock;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wordesign-test-{}-{}", std::process::id(), tag));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn default_names_count_up_and_skip_taken() {
        let d = tmp("names");
        assert_eq!(default_stem(&d, &[]), "prompt_1");
        std::fs::write(path_for(&d, "prompt_1"), "").unwrap();
        assert_eq!(default_stem(&d, &[]), "prompt_2");
        // an unsaved in-memory prompt also reserves its name
        assert_eq!(default_stem(&d, &["prompt_2"]), "prompt_3");
        std::fs::write(path_for(&d, "prompt_3"), "").unwrap();
        assert_eq!(default_stem(&d, &[]), "prompt_2");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn sanitize_blocks_path_tricks_and_reserved_names() {
        assert_eq!(sanitize("my/prompt:1?"), "my_prompt_1_");
        assert_eq!(sanitize("  ..hidden.. "), "hidden");
        assert_eq!(sanitize("../../etc/passwd"), "_.._etc_passwd");
        assert_eq!(sanitize("   "), "");
        assert_eq!(sanitize("con"), "con_");
        assert_eq!(sanitize("COM3"), "COM3_");
        assert_eq!(sanitize("COM10"), "COM10");
        assert_eq!(sanitize(&"x".repeat(200)).chars().count(), 80);
        assert_eq!(sanitize("résumé ✨"), "résumé ✨");
    }

    #[test]
    fn unique_path_avoids_clobbering_but_allows_self() {
        let d = tmp("unique");
        let a = unique_path(&d, "note", None);
        assert_eq!(a, path_for(&d, "note"));
        std::fs::write(&a, "x").unwrap();
        assert_eq!(unique_path(&d, "note", None), path_for(&d, "note_2"));
        assert_eq!(unique_path(&d, "note", Some(&a)), a);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn save_load_list_roundtrip() {
        let d = tmp("roundtrip");
        let mut p = Prompt::new();
        p.body = vec![BodyBlock::Text("hello wordesign".into())];
        let path = unique_path(&d, "prompt_1", None);
        write_atomic(&path, &p.to_file()).unwrap();
        assert!(!path.with_extension("aiprompt.tmp").exists(), "temp file cleaned up");
        std::fs::write(d.join("ignored.txt"), "nope").unwrap();
        let entries = list(&d);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "prompt_1");
        let q = load(&path).unwrap();
        assert_eq!(q.to_file(), p.to_file());
        // overwrite in place
        p.body = vec![BodyBlock::Text("changed".into())];
        write_atomic(&path, &p.to_file()).unwrap();
        assert_eq!(load(&path).unwrap().to_file(), p.to_file());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn long_non_latin_names_stay_within_filesystem_limits() {
        let cjk = sanitize(&"中".repeat(100));
        assert!(cjk.len() <= 180 && cjk.chars().count() <= 80, "{}", cjk.len());
        let emoji = sanitize(&"😀".repeat(100));
        assert!(emoji.len() <= 180);
        // and such a name can really be written
        let d = tmp("longname");
        let p = unique_path(&d, &cjk, None);
        write_atomic(&p, "x").unwrap();
        assert!(p.exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn unicode_names_work_for_prompts_and_folders() {
        let d = tmp("unicode");
        let folder = create_folder(&d, "مجلد 文件夹").unwrap();
        assert_eq!(folder, "مجلد 文件夹");
        let path = unique_path(&folder_dir(&d, Some(&folder)), "提示词 پرامپت", None);
        let mut p = Prompt::new();
        p.body = vec![BodyBlock::Text("你好，世界 — مرحبا بالعالم".into())];
        write_atomic(&path, &p.to_file()).unwrap();
        let lib = scan(&d);
        assert_eq!(lib.folders, vec!["مجلد 文件夹".to_string(), TEMP.to_string()]);
        assert_eq!(lib.entries[0].name, "提示词 پرامپت");
        assert_eq!(lib.entries[0].folder.as_deref(), Some("مجلد 文件夹"));
        assert_eq!(load(&path).unwrap().to_file(), p.to_file());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn scan_lists_folders_including_empty_ones_and_skips_hidden() {
        let d = tmp("scan");
        std::fs::write(path_for(&d, "loose"), "").unwrap();
        std::fs::create_dir_all(d.join("Work")).unwrap();
        std::fs::create_dir_all(d.join("archive")).unwrap();
        std::fs::create_dir_all(d.join(".hidden")).unwrap();
        std::fs::write(path_for(&d.join("Work"), "a"), "").unwrap();
        std::fs::write(path_for(&d.join(".hidden"), "secret"), "").unwrap();
        let lib = scan(&d);
        assert_eq!(lib.folders, vec!["archive".to_string(), "Work".to_string(), TEMP.to_string()]);
        let mut names: Vec<(String, Option<String>)> =
            lib.entries.iter().map(|e| (e.name.clone(), e.folder.clone())).collect();
        names.sort();
        assert_eq!(
            names,
            vec![("a".to_string(), Some("Work".to_string())), ("loose".to_string(), None)]
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn create_folder_is_idempotent_and_case_insensitive() {
        let d = tmp("mkfolder");
        assert_eq!(create_folder(&d, "Work").unwrap(), "Work");
        assert_eq!(create_folder(&d, "work").unwrap(), "Work");
        assert_eq!(create_folder(&d, " Work ").unwrap(), "Work");
        assert_eq!(create_folder(&d, "a/b").unwrap(), "a_b");
        assert!(create_folder(&d, "   ").is_err());
        assert!(create_folder(&d, "..").is_err());
        assert_eq!(scan(&d).folders.len(), 3);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn moving_between_folders_keeps_content_and_never_clobbers() {
        let d = tmp("move");
        let work = create_folder(&d, "Work").unwrap();
        let a = unique_path(&d, "prompt_1", None);
        write_atomic(&a, "one").unwrap();
        // an unrelated prompt with the same name already sits in the folder
        let clash = unique_path(&folder_dir(&d, Some(&work)), "prompt_1", None);
        write_atomic(&clash, "other").unwrap();

        let moved = move_to_folder(&d, &a, Some(&work)).unwrap();
        assert_eq!(moved, folder_dir(&d, Some(&work)).join("prompt_1_2.aiprompt"));
        assert_eq!(std::fs::read_to_string(&moved).unwrap(), "one");
        assert_eq!(std::fs::read_to_string(&clash).unwrap(), "other");
        assert!(!a.exists());
        assert_eq!(folder_of(&d, &moved).as_deref(), Some("Work"));

        // and back to the top level
        let back = move_to_folder(&d, &moved, None).unwrap();
        // it keeps the name it was given when it had to dodge the clash
        assert_eq!(back, d.join("prompt_1_2.aiprompt"));
        assert_eq!(folder_of(&d, &back), None);
        // moving to where it already is is a no-op
        assert_eq!(move_to_folder(&d, &back, None).unwrap(), back);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn retarget_does_not_touch_the_disk() {
        let d = tmp("retarget");
        let p = d.join("x.aiprompt");
        let t = retarget(&d, &p, Some("NotCreatedYet"));
        assert_eq!(t, d.join("NotCreatedYet").join("x.aiprompt"));
        assert!(!d.join("NotCreatedYet").exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn temp_purge_only_removes_old_prompts_and_never_the_open_one() {
        let base = tmp("purge");
        let t = base.join(TEMP);
        std::fs::create_dir_all(&t).unwrap();
        for n in ["a", "b", "open"] {
            std::fs::write(t.join(format!("{n}.aiprompt")), "x").unwrap();
        }
        std::fs::write(t.join("notes.txt"), "x").unwrap();
        std::fs::write(base.join("loose.aiprompt"), "x").unwrap();
        // nothing is older than a day yet
        assert_eq!(purge_temp(&base, std::time::Duration::from_secs(86_400), None), 0);
        // with a zero max age everything but the open prompt (and non-prompts) goes
        std::thread::sleep(std::time::Duration::from_millis(30));
        let open = t.join("open.aiprompt");
        assert_eq!(purge_temp(&base, std::time::Duration::from_millis(1), Some(&open)), 2);
        assert!(open.exists() && t.join("notes.txt").exists() && base.join("loose.aiprompt").exists());
    }

    #[test]
    fn scan_always_lists_temp_last() {
        let base = tmp("scan-temp");
        std::fs::create_dir_all(base.join("Zeta")).unwrap();
        std::fs::create_dir_all(base.join("Alpha")).unwrap();
        assert_eq!(scan(&base).folders, vec!["Alpha", "Zeta", TEMP]);
        std::fs::create_dir_all(base.join(TEMP)).unwrap();
        assert_eq!(scan(&base).folders, vec!["Alpha", "Zeta", TEMP]);
    }

    #[test]
    fn deleting_folders_is_contained_and_protects_temp() {
        let base = tmp("delete");
        std::fs::create_dir_all(base.join("Work")).unwrap();
        std::fs::write(base.join("Work/p.aiprompt"), "x").unwrap();
        std::fs::write(base.join("keep.aiprompt"), "x").unwrap();
        std::fs::create_dir_all(base.join(TEMP)).unwrap();
        assert!(delete_folder(&base, TEMP).is_err());
        assert!(delete_folder(&base, "..").is_err());
        assert!(delete_folder(&base, "a/b").is_err());
        assert!(delete_folder(&base, "Nope").is_err());
        delete_folder(&base, "Work").unwrap();
        assert!(!base.join("Work").exists());
        assert!(base.join("keep.aiprompt").exists() && base.join(TEMP).exists());
    }

    #[test]
    fn reusable_contexts_round_trip_and_stay_out_of_the_library() {
        let base = tmp("contexts");
        assert!(list_contexts(&base).is_empty());
        assert!(write_context(&base, "  ", "x").is_err());
        assert_eq!(write_context(&base, "Pirate / voice", "Arr.\n\nSecond").unwrap(), "Pirate _ voice");
        write_context(&base, "apple", "A").unwrap();
        write_context(&base, "apple", "A2").unwrap();
        assert_eq!(list_contexts(&base), ["apple", "Pirate _ voice"]);
        assert_eq!(read_context(&base, "apple").unwrap(), "A2");
        assert_eq!(read_context(&base, "Pirate _ voice").unwrap(), "Arr.\n\nSecond");
        let lib = scan(&base);
        assert!(lib.entries.is_empty() && lib.folders == vec![TEMP.to_string()], "hidden from the sidebar");
        delete_context(&base, "apple").unwrap();
        assert_eq!(list_contexts(&base), ["Pirate _ voice"]);
    }

    #[test]
    fn delete_prompt_only_removes_prompt_files_inside_the_library() {
        let base = tmp("delprompt");
        let other = tmp("delprompt-outside");
        std::fs::write(base.join("a.aiprompt"), "x").unwrap();
        std::fs::write(base.join("note.txt"), "x").unwrap();
        std::fs::write(other.join("b.aiprompt"), "x").unwrap();
        assert!(delete_prompt(&base, &base.join("note.txt")).is_err());
        assert!(delete_prompt(&base, &other.join("b.aiprompt")).is_err());
        assert!(delete_prompt(&base, &base.join("../x.aiprompt")).is_err());
        assert!(delete_prompt(&base, &base.join("missing.aiprompt")).is_err());
        delete_prompt(&base, &base.join("a.aiprompt")).unwrap();
        assert!(!base.join("a.aiprompt").exists() && base.join("note.txt").exists() && other.join("b.aiprompt").exists());
    }
}
