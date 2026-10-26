//! Taught drawings on disk, one `<name>.json` per drawing in
//! `$XDG_DATA_HOME/air-gestures/drawings` (else `~/.local/share/air-gestures/drawings`):
//! `{"version": 1, "frame": 2, "tries": [[[[x, y], …], …], …]}`, each try a list of strokes,
//! each stroke a list of [x, y] points in degrees (band-core's `draw::Stroke`). `frame` is the
//! drawing frame the points are in: 2 since band-core turned its drawings the right way up
//! (2026-10-26); a template without one (1) is turned over (negated) when it's loaded.

use std::fs::File;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use band_core::controls::draw::{self, Drawing, Stroke, TeachError};
use serde::{Deserialize, Serialize};

const VERSION: u32 = 1;
/// band-core's drawing frame since 2026-10-26: the old projection negated.
const FRAME: u32 = 2;

fn old_frame() -> u32 {
    1
}

#[derive(Serialize, Deserialize)]
struct Template {
    version: u32,
    #[serde(default = "old_frame")]
    frame: u32,
    tries: Vec<Vec<Stroke>>,
}

/// What `load_all` found.
#[derive(Debug, Default)]
pub struct Loaded {
    pub drawings: Vec<Drawing>,
    /// What couldn't be read (or reads badly), described.
    pub problems: Vec<String>,
    /// Drawings taught in the old frame, turned over (and rewritten) just now.
    pub migrated: Vec<String>,
}

/// From the old frame: every point negated (exact for drawings taught without a palm
/// calibration).
fn turned_over(tries: Vec<Vec<Stroke>>) -> Vec<Vec<Stroke>> {
    tries.into_iter().map(|strokes| strokes.into_iter().map(|stroke| stroke.into_iter().map(|[x, y]| [-x, -y]).collect()).collect()).collect()
}

/// Rewrite an old-frame template in the new frame, keeping the original as `<name>.json.bak`
/// (once).
fn migrate(dir: &Path, drawing: &Drawing) -> io::Result<()> {
    let backup = dir.join(format!("{}.json.bak", drawing.name));
    if !backup.exists() {
        std::fs::copy(path(dir, &drawing.name), &backup)?;
    }
    save(dir, drawing)
}

/// The directory (the dry-run daemon's is separate, like its state).
pub fn default_dir(dry_run: bool) -> PathBuf {
    let base = crate::config::xdg("XDG_DATA_HOME").unwrap_or_else(|| crate::config::home().join(".local/share"));
    base.join(if dry_run { "air-gestures-dry-run" } else { "air-gestures" }).join("drawings")
}

/// Drawing names are file names and go into `AIR_GESTURES_DRAWING='…'`.
pub fn check_name(name: &str) -> Result<(), String> {
    crate::config::check_name("drawing", name)
}

fn path(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}.json"))
}

fn invalid(error: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, error)
}

/// A `save`'s temp file (`.<name>.json.tmp`), while it runs or after it was interrupted.
fn temp_name(file: &str) -> Option<&str> {
    file.strip_prefix('.')?.strip_suffix(".json.tmp")
}

/// Removes the temp files interrupted saves left in `dir`, those older than `older_than` (a
/// younger one may belong to a save under way, in this process or another). The daemon does
/// it when it starts. Returns what couldn't be removed.
pub fn remove_stale(dir: &Path, older_than: Duration) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let now = SystemTime::now();
    let mut problems = Vec::new();
    for entry in entries.flatten() {
        let file = entry.path();
        if file.file_name().and_then(|n| n.to_str()).and_then(temp_name).is_none() {
            continue;
        }
        let modified = entry.metadata().and_then(|m| m.modified()).unwrap_or(now);
        if now.duration_since(modified).unwrap_or_default() < older_than {
            continue;
        }
        if let Err(error) = std::fs::remove_file(&file)
            && error.kind() != io::ErrorKind::NotFound
        {
            problems.push(format!("{}: {error}", file.display()));
        }
    }
    problems
}

/// Every drawing in `dir`, by name; what couldn't be read (or reads badly), described. No
/// directory: none. A save's temp file is skipped (see `remove_stale`). See `load_all`.
pub fn load(dir: &Path) -> (Vec<Drawing>, Vec<String>) {
    let loaded = load_all(dir);
    (loaded.drawings, loaded.problems)
}

/// `load`, and which old-frame templates were turned over: each is rewritten in the new frame
/// (the original kept as `<name>.json.bak`); one that can't be rewritten is still used, turned
/// over, with a problem saying to teach it again.
pub fn load_all(dir: &Path) -> Loaded {
    let mut loaded = Loaded::default();
    let Loaded { drawings, problems, migrated } = &mut loaded;
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return loaded,
        Err(error) => {
            problems.push(format!("{}: {error}", dir.display()));
            return loaded;
        }
    };
    for entry in entries.flatten() {
        let file = entry.path();
        let Some(file_name) = file.file_name().and_then(|n| n.to_str()) else { continue };
        if temp_name(file_name).is_some() {
            continue;
        }
        let Some(name) = file_name.strip_suffix(".json") else { continue };
        if let Err(error) = check_name(name) {
            problems.push(format!("{}: not a drawing's name ({error})", file.display()));
            continue;
        }
        let read = std::fs::read(&file)
            .map_err(|e| e.to_string())
            .and_then(|bytes| serde_json::from_slice::<Template>(&bytes).map_err(|e| e.to_string()));
        match read {
            Ok(f) if f.version == VERSION && f.frame > FRAME => {
                problems.push(format!("{}: frame {} (this air-gesturesd reads {FRAME})", file.display(), f.frame));
            }
            Ok(mut f) if f.version == VERSION => {
                if f.frame < FRAME {
                    let drawing = Drawing { name: name.to_owned(), tries: turned_over(std::mem::take(&mut f.tries)) };
                    match migrate(dir, &drawing) {
                        Ok(()) => migrated.push(name.to_owned()),
                        Err(error) => problems.push(format!(
                            "{}: taught before the drawing frame changed and can't be updated ({error}); it's used turned over for now: teach {name} again",
                            file.display()
                        )),
                    }
                    f.tries = drawing.tries;
                }
                match draw::check_tries(&f.tries) {
                    Err(TeachError::TooSmall(i)) => problems.push(format!(
                        "{}: try {} can't be read; teach it again (`air-gestures draw delete {name}`, then `draw teach {name}`)",
                        file.display(),
                        i + 1
                    )),
                    Err(TeachError::NoTries) => {
                        problems.push(format!("{}: no tries; teach it again (`air-gestures draw teach {name}`)", file.display()))
                    }
                    Ok(()) | Err(TeachError::NotAlike { .. }) => {}
                }
                drawings.push(Drawing { name: name.to_owned(), tries: f.tries });
            }
            Ok(f) => problems.push(format!("{}: version {} (this air-gesturesd reads {VERSION})", file.display(), f.version)),
            Err(error) => problems.push(format!("{}: {error}", file.display())),
        }
    }
    drawings.sort_by(|a, b| a.name.cmp(&b.name));
    migrated.sort();
    loaded
}

/// Write it atomically and durably (a temp file, fsynced, renamed into place, then the
/// directory fsynced), creating `dir` if needed.
pub fn save(dir: &Path, drawing: &Drawing) -> io::Result<()> {
    check_name(&drawing.name).map_err(invalid)?;
    std::fs::create_dir_all(dir)?;
    let json = serde_json::to_vec(&Template { version: VERSION, frame: FRAME, tries: drawing.tries.clone() }).map_err(io::Error::other)?;
    let temp = dir.join(format!(".{}.json.tmp", drawing.name));
    {
        let mut file = File::create(&temp)?;
        file.write_all(&json)?;
        file.sync_all()?;
    }
    std::fs::rename(&temp, path(dir, &drawing.name))?;
    File::open(dir)?.sync_all()
}

/// Remove it; `false` when there was none.
pub fn delete(dir: &Path, name: &str) -> io::Result<bool> {
    check_name(name).map_err(invalid)?;
    match std::fs::remove_file(path(dir, name)) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

/// Changes whenever a drawing is saved or deleted: the directory's modification time. A file
/// edited in place (not through `save`) doesn't change it: that edit is read at the next
/// save or delete, or when the daemon starts.
pub fn stamp(dir: &Path) -> Option<SystemTime> {
    std::fs::metadata(dir).and_then(|m| m.modified()).ok()
}
