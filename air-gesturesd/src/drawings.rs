//! Taught drawings on disk, one `<name>.json` per drawing in
//! `$XDG_DATA_HOME/air-gestures/drawings` (else `~/.local/share/air-gestures/drawings`):
//! `{"version": 1, "tries": [[[[x, y], …], …], …]}`, each try a list of strokes, each stroke
//! a list of [x, y] points in degrees (band-core's `draw::Stroke`).

use std::fs::File;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use band_core::controls::draw::{self, Drawing, Stroke, TeachError};
use serde::{Deserialize, Serialize};

const VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct Template {
    version: u32,
    tries: Vec<Vec<Stroke>>,
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

/// The temp file an interrupted `save` of `name` leaves behind.
fn temp_name(file: &str) -> Option<&str> {
    file.strip_prefix('.')?.strip_suffix(".json.tmp")
}

/// Every drawing in `dir`, by name; what couldn't be read (or reads badly), described. No
/// directory: none. Cleans up what an interrupted save left behind.
pub fn load(dir: &Path) -> (Vec<Drawing>, Vec<String>) {
    let (mut drawings, mut problems) = (Vec::new(), Vec::new());
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return (drawings, problems),
        Err(error) => {
            problems.push(format!("{}: {error}", dir.display()));
            return (drawings, problems);
        }
    };
    for entry in entries.flatten() {
        let file = entry.path();
        let Some(file_name) = file.file_name().and_then(|n| n.to_str()) else { continue };
        if temp_name(file_name).is_some() {
            if let Err(error) = std::fs::remove_file(&file) {
                problems.push(format!("{}: {error}", file.display()));
            }
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
            Ok(f) if f.version == VERSION => {
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
    (drawings, problems)
}

/// Write it atomically and durably (a temp file, fsynced, renamed into place, then the
/// directory fsynced), creating `dir` if needed.
pub fn save(dir: &Path, drawing: &Drawing) -> io::Result<()> {
    check_name(&drawing.name).map_err(invalid)?;
    std::fs::create_dir_all(dir)?;
    let json = serde_json::to_vec(&Template { version: VERSION, tries: drawing.tries.clone() }).map_err(io::Error::other)?;
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
