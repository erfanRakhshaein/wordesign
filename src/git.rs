//! Version control for single prompts, using the local `git` command line.
//!
//! One repository lives in the library folder (`~/.wordesign`). A prompt is only ever added to it when
//! the user ticks "track"; the list of tracked prompts is kept in `.tracked`. Commits always name
//! their file(s) explicitly, so they never sweep in other prompts.

use std::path::{Path, PathBuf};
use std::process::Command;

const TRACKED_FILE: &str = ".tracked";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Version {
    pub hash: String,
    /// Unix seconds.
    pub when: i64,
    pub subject: String,
    /// Commit time in the computer's local time zone, `2026-10-25 16:49`.
    pub local: String,
    /// Path of the file (relative to the library) as it was named in that commit.
    pub path: String,
}

fn git(base: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .current_dir(base)
        .env("GIT_TERMINAL_PROMPT", "0")
        .args([
            "-c",
            "user.name=wordesign",
            "-c",
            "user.email=wordesign@localhost",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.quotepath=false",
        ])
        .args(args)
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                "git was not found on this computer — install git to use version control".to_owned()
            } else {
                format!("could not run git: {e}")
            }
        })?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        let msg = err.trim();
        Err(if msg.is_empty() {
            String::from_utf8_lossy(&out.stdout).trim().to_owned()
        } else {
            msg.to_owned()
        })
    }
}

#[allow(dead_code)]
pub fn available() -> bool {
    Command::new("git").arg("--version").output().is_ok_and(|o| o.status.success())
}

/// Path relative to the library, always with `/`.
pub fn rel(base: &Path, path: &Path) -> String {
    path.strip_prefix(base)
        .unwrap_or(path)
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

fn ensure_repo(base: &Path) -> Result<(), String> {
    if base.join(".git").exists() {
        return Ok(());
    }
    git(base, &["init", "-q"]).map(|_| ())
}

pub fn tracked_list(base: &Path) -> Vec<String> {
    std::fs::read_to_string(base.join(TRACKED_FILE))
        .map(|t| t.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_owned).collect())
        .unwrap_or_default()
}

fn save_list(base: &Path, list: &[String]) {
    let _ = std::fs::write(base.join(TRACKED_FILE), list.join("\n") + "\n");
}

#[allow(dead_code)]
pub fn is_tracked(base: &Path, path: &Path) -> bool {
    let r = rel(base, path);
    tracked_list(base).iter().any(|t| *t == r)
}

/// Start tracking `path` (the file must exist) and record its current content as the first version.
pub fn track(base: &Path, path: &Path) -> Result<(), String> {
    ensure_repo(base)?;
    let r = rel(base, path);
    let mut list = tracked_list(base);
    if !list.contains(&r) {
        list.push(r.clone());
        save_list(base, &list);
    }
    git(base, &["add", "--", &r])?;
    if is_dirty(base, path) {
        let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        git(base, &["commit", "-q", "-m", &format!("Start tracking {name}"), "--only", "--", &r])?;
    }
    Ok(())
}

/// Stop tracking (history stays in the repository; ticking again continues it).
pub fn untrack(base: &Path, path: &Path) {
    let r = rel(base, path);
    let list: Vec<String> = tracked_list(base).into_iter().filter(|t| *t != r).collect();
    save_list(base, &list);
}

/// Forget tracked entries whose file no longer exists (deleted prompts / folders).
pub fn prune(base: &Path) {
    let list = tracked_list(base);
    let kept: Vec<String> = list.iter().filter(|r| base.join(r).is_file()).cloned().collect();
    if kept.len() != list.len() {
        save_list(base, &kept);
    }
}

/// Does `path` have changes since its last commit (or was never committed)?
pub fn is_dirty(base: &Path, path: &Path) -> bool {
    let r = rel(base, path);
    git(base, &["status", "--porcelain", "--", &r]).map(|s| !s.trim().is_empty()).unwrap_or(false)
}

/// Commit only this prompt. `Ok(false)` when there was nothing new to commit.
pub fn commit(base: &Path, path: &Path, message: &str) -> Result<bool, String> {
    ensure_repo(base)?;
    let r = rel(base, path);
    git(base, &["add", "--", &r])?;
    if !is_dirty(base, path) {
        return Ok(false);
    }
    let msg = if message.trim().is_empty() { "Update" } else { message.trim() };
    git(base, &["commit", "-q", "-m", msg, "--only", "--", &r])?;
    Ok(true)
}

/// A tracked prompt was renamed / moved on disk: carry its history along.
pub fn rename(base: &Path, old: &Path, new: &Path) -> Result<(), String> {
    let (ro, rn) = (rel(base, old), rel(base, new));
    let mut list = tracked_list(base);
    let Some(pos) = list.iter().position(|t| *t == ro) else { return Ok(()) };
    list[pos] = rn.clone();
    save_list(base, &list);
    git(base, &["add", "-A", "--", &ro, &rn])?;
    if git(base, &["status", "--porcelain", "--", &ro, &rn]).is_ok_and(|s| !s.trim().is_empty()) {
        git(
            base,
            &["commit", "-q", "-m", &format!("Move {ro} to {rn}"), "--only", "--", &ro, &rn],
        )?;
    }
    Ok(())
}

/// Previous versions, newest first.
pub fn history(base: &Path, path: &Path) -> Result<Vec<Version>, String> {
    let r = rel(base, path);
    let out = git(
        base,
        &[
            "log",
            "--follow",
            "--date=format-local:%Y-%m-%d %H:%M",
            "--format=%x1e%H%x1f%ct%x1f%s%x1f%cd",
            "--name-only",
            "--",
            &r,
        ],
    )?;
    let mut v = Vec::new();
    for rec in out.split('\x1e').filter(|r| !r.trim().is_empty()) {
        let mut lines = rec.lines();
        let Some(head) = lines.next() else { continue };
        let mut f = head.split('\x1f');
        let (Some(hash), Some(ts), Some(subject), Some(local)) = (f.next(), f.next(), f.next(), f.next()) else { continue };
        let p = lines.find(|l| !l.trim().is_empty()).unwrap_or(&r).to_owned();
        v.push(Version {
            hash: hash.to_owned(),
            when: ts.parse().unwrap_or(0),
            subject: subject.to_owned(),
            local: local.to_owned(),
            path: p,
        });
    }
    Ok(v)
}

pub fn content_at(base: &Path, v: &Version) -> Result<String, String> {
    git(base, &["show", &format!("{}:{}", v.hash, v.path)])
}

/// Restore an old version into the file. The current state is committed first (if it has changes),
/// and the restore itself becomes a new commit, so nothing is ever lost.
pub fn rollback(base: &Path, path: &Path, v: &Version) -> Result<(), String> {
    let old = content_at(base, v)?;
    commit(base, path, "Before rollback")?;
    std::fs::write(path, old).map_err(|e| e.to_string())?;
    let label = if v.local.is_empty() { short_date(v.when) } else { v.local.clone() };
    commit(base, path, &format!("Rollback to {label}: {}", v.subject))?;
    Ok(())
}

/// `2026-10-25 14:02` (UTC; good enough to tell versions apart).
pub fn short_date(unix: i64) -> String {
    let days = unix.div_euclid(86_400);
    let secs = unix.rem_euclid(86_400);
    // civil-from-days (Howard Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}", secs / 3600, (secs % 3600) / 60)
}

#[allow(dead_code)]
pub fn repo_path(base: &Path) -> PathBuf {
    base.join(".git")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wd-git-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn date_formatting() {
        assert_eq!(short_date(0), "1970-01-01 00:00");
        assert_eq!(short_date(1_761_400_920), "2025-10-25 14:02");
    }

    #[test]
    fn track_commit_history_and_rollback() {
        if !available() {
            return;
        }
        let base = tmp("flow");
        let a = base.join("a.aiprompt");
        let b = base.join("b.aiprompt");
        std::fs::write(&a, "one").unwrap();
        std::fs::write(&b, "other").unwrap();
        track(&base, &a).unwrap();
        assert!(is_tracked(&base, &a) && !is_tracked(&base, &b));
        assert!(!is_dirty(&base, &a));

        std::fs::write(&a, "two").unwrap();
        std::fs::write(&b, "other changed").unwrap();
        assert!(is_dirty(&base, &a));
        assert_eq!(commit(&base, &a, "second").unwrap(), true);
        assert_eq!(commit(&base, &a, "again").unwrap(), false, "nothing new");
        // only a was committed, b was never added
        let tree = git(&base, &["ls-files"]).unwrap();
        assert!(tree.contains("a.aiprompt") && !tree.contains("b.aiprompt"));

        let h = history(&base, &a).unwrap();
        assert_eq!(h.iter().map(|v| v.subject.as_str()).collect::<Vec<_>>(), ["second", "Start tracking a"]);
        assert_eq!(content_at(&base, &h[1]).unwrap(), "one");

        std::fs::write(&a, "three, unsaved in git").unwrap();
        rollback(&base, &a, &h[1]).unwrap();
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "one");
        let subjects: Vec<String> = history(&base, &a).unwrap().into_iter().map(|v| v.subject).collect();
        assert!(subjects[0].starts_with("Rollback to") && subjects[1] == "Before rollback", "{subjects:?}");
        // the unsaved state is recoverable
        let before = history(&base, &a).unwrap().remove(1);
        assert_eq!(content_at(&base, &before).unwrap(), "three, unsaved in git");
    }

    #[test]
    fn renames_keep_history() {
        if !available() {
            return;
        }
        let base = tmp("rename");
        let a = base.join("a.aiprompt");
        std::fs::write(&a, "hello world, this is a reasonably long prompt body").unwrap();
        track(&base, &a).unwrap();
        let n = base.join("new_name.aiprompt");
        std::fs::rename(&a, &n).unwrap();
        rename(&base, &a, &n).unwrap();
        assert!(is_tracked(&base, &n) && !is_tracked(&base, &a));
        let h = history(&base, &n).unwrap();
        assert!(h.len() >= 2, "history follows the rename: {h:?}");
        assert_eq!(content_at(&base, h.last().unwrap()).unwrap(), "hello world, this is a reasonably long prompt body");
        untrack(&base, &n);
        assert!(!is_tracked(&base, &n));
    }
}
