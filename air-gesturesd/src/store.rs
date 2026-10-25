//! Owner keys, the paired band, the Meta session and, per band and hand, what was taught
//! (`calibration.json`), under
//! `$XDG_STATE_HOME/air-gestures` (else `~/.local/state/air-gestures`): a 0700 directory
//! of 0600 files, each written atomically and fsynced. Nothing here is logged.

use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use band_core::controls::pinch::{Prototypes, TaughtError};
use band_core::controls::PalmCalibration;
use band_core::events::Hand;
use band_core::identity::EnrollmentIdentity;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

const OWNER: &str = "owner.key";
const PENDING_OWNER: &str = "owner.pending.key";
const BAND: &str = "band.json";
pub(crate) const META_SESSION: &str = "meta-session.json";
/// `{"version": 1, "bands": {address: {"left"|"right": HandCalibration}}}`.
const CALIBRATION: &str = "calibration.json";
const CALIBRATION_VERSION: u64 = 1;
/// Pre-single-file-identity layout; `forget()` still cleans these up if found.
const LEGACY_BAND_KEY: &str = "band.pub";
const LEGACY_PENDING_BAND_KEY: &str = "band.pending.pub";

/// The paired band: its Bluetooth address and advertised name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BandRecord {
    pub address: String,
    pub name: String,
    /// Which `SCHEME_26_GUESSES` entry a newer-firmware band answered to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheme_guess: Option<usize>,
}

/// Which identity file `Store::connect_identity` returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentitySource {
    /// A claim the band may have committed, whose confirmation never arrived.
    Pending,
    Confirmed,
}

/// An owner key file's bytes: the 32-byte private key, optionally followed by
/// the 65-byte X9.63 band key (32 or 97 bytes total; anything else is corrupt).
pub fn identity_bytes(identity: &EnrollmentIdentity) -> Vec<u8> {
    let mut bytes = identity.private_bytes().to_vec();
    if let Some(band_key) = identity.band_key_x963() {
        bytes.extend(band_key);
    }
    bytes
}

/// The inverse of [`identity_bytes`]; the error never contains key material.
pub fn identity_from_bytes(bytes: &[u8]) -> Result<EnrollmentIdentity, String> {
    let (private, band) = match bytes.len() {
        32 => (&bytes[..32], None),
        97 => (&bytes[..32], Some(&bytes[32..])),
        length => return Err(format!("invalid identity file length ({length} bytes)")),
    };
    EnrollmentIdentity::from_bytes(private, band).map_err(|error| error.to_string())
}

/// Palm down and up, as the band's roll about the forearm (degrees).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct StoredPalm {
    pub down_roll: f64,
    pub up_roll: f64,
}

impl StoredPalm {
    pub fn calibration(&self) -> PalmCalibration {
        PalmCalibration { down_roll: self.down_roll, up_roll: self.up_roll }
    }
}

impl From<PalmCalibration> for StoredPalm {
    fn from(c: PalmCalibration) -> Self {
        Self { down_roll: c.down_roll, up_roll: c.up_roll }
    }
}

/// Taught drag directions (forearm frame, unit): the turn, then the six drags in
/// `Drag::ALL` order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredDrags {
    pub palm_down_roll: f64,
    pub turn: [f64; 3],
    pub drags: [[f64; 3]; 6],
}

impl StoredDrags {
    /// `None` for turns-only prototypes (nothing taught).
    pub fn from_prototypes(p: &Prototypes) -> Option<Self> {
        Some(Self { palm_down_roll: p.palm_down_roll, turn: p.turn, drags: p.drags? })
    }

    /// Checked again on the way in: a hand-edited file can't make lookalike drags.
    pub fn prototypes(&self) -> Result<Prototypes, TaughtError> {
        Prototypes::taught(self.turn, self.drags, self.palm_down_roll)
    }
}

/// What was taught on one band for one hand.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct HandCalibration {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub palm: Option<StoredPalm>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drags: Option<StoredDrags>,
}

impl HandCalibration {
    pub fn is_empty(&self) -> bool {
        self.palm.is_none() && self.drags.is_none()
    }

    /// What goes into `Controls::set_calibration` for the hand in use (nothing taught: the
    /// core's defaults), each checked again on the way in (a hand-edited file can't make a
    /// calibration the daemon would refuse); and what couldn't be used.
    pub fn apply(&self) -> (Option<PalmCalibration>, Option<Prototypes>, Vec<String>) {
        let mut problems = Vec::new();
        let palm = self.palm.and_then(|p| match PalmCalibration::from_rolls(&[p.down_roll], &[p.up_roll]) {
            Ok(_) => Some(p.calibration()),
            Err(error) => {
                problems.push(format!(
                    "the palm calibration can't be used ({}); calibrate again with `air-gestures calibrate`",
                    crate::teach::calibration_error_text(error)
                ));
                None
            }
        });
        let drags = match self.drags.as_ref().map(StoredDrags::prototypes) {
            None => None,
            Some(Ok(prototypes)) => Some(prototypes),
            Some(Err(error)) => {
                problems.push(format!(
                    "the taught drags can't be used ({}); teach them again with `air-gestures teach-drags`",
                    crate::teach::taught_error_text(error)
                ));
                None
            }
        };
        (palm, drags, problems)
    }
}

fn hand_key(hand: Hand) -> &'static str {
    match hand {
        Hand::Left => "left",
        Hand::Right => "right",
    }
}

#[derive(Debug, Clone)]
pub struct Store {
    dir: PathBuf,
}

impl Store {
    pub fn open(dir: impl Into<PathBuf>) -> io::Result<Self> {
        let dir = dir.into();
        DirBuilder::new().recursive(true).mode(0o700).create(&dir)?;
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
        Ok(Self { dir })
    }

    /// `$XDG_STATE_HOME/air-gestures`, else `~/.local/state/air-gestures`.
    pub fn default_dir() -> PathBuf {
        crate::config::xdg("XDG_STATE_HOME")
            .unwrap_or_else(|| crate::config::home().join(".local/state"))
            .join("air-gestures")
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    fn remove_path(path: &Path) -> io::Result<()> {
        match fs::remove_file(path) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        }
    }

    /// Fsync the directory itself, so a rename or unlink survives a crash.
    fn sync_dir(&self) -> io::Result<()> {
        File::open(&self.dir)?.sync_all()
    }

    pub(crate) fn write_secret(&self, name: &str, bytes: &[u8]) -> io::Result<()> {
        let temp = self.path(&format!(".{name}.tmp"));
        Self::remove_path(&temp)?;
        {
            let mut file = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&temp)?;
            file.set_permissions(fs::Permissions::from_mode(0o600))?;
            file.write_all(bytes)?;
            file.sync_all()?;
        }
        fs::rename(&temp, self.path(name))?;
        self.sync_dir()
    }

    pub(crate) fn read(&self, name: &str) -> io::Result<Option<Vec<u8>>> {
        match fs::read(self.path(name)) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    pub(crate) fn remove(&self, name: &str) -> io::Result<()> {
        Self::remove_path(&self.path(name))?;
        self.sync_dir()
    }

    fn write_identity(&self, name: &str, identity: &EnrollmentIdentity) -> io::Result<()> {
        self.write_secret(name, &identity_bytes(identity))
    }

    fn read_identity(&self, name: &str) -> io::Result<Option<EnrollmentIdentity>> {
        let Some(bytes) = self.read(name)? else {
            return Ok(None);
        };
        identity_from_bytes(&bytes)
            .map(Some)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, format!("{name}: {error}")))
    }

    /// Store the confirmed owner identity and drop any pending one.
    pub fn save_identity(&self, identity: &EnrollmentIdentity) -> io::Result<()> {
        self.write_identity(OWNER, identity)?;
        self.remove(PENDING_OWNER)
    }

    pub fn load_identity(&self) -> io::Result<Option<EnrollmentIdentity>> {
        self.read_identity(OWNER)
    }

    /// Store the identity the band may commit to before its confirmation arrives.
    pub fn save_pending(&self, identity: &EnrollmentIdentity) -> io::Result<()> {
        self.write_identity(PENDING_OWNER, identity)
    }

    pub fn load_pending(&self) -> io::Result<Option<EnrollmentIdentity>> {
        self.read_identity(PENDING_OWNER)
    }

    /// Whether a pending identity file is there at all (readable or not).
    pub fn has_pending(&self) -> bool {
        self.path(PENDING_OWNER).exists()
    }

    /// Drop the pending identity without confirming it.
    pub fn discard_pending(&self) -> io::Result<()> {
        self.remove(PENDING_OWNER)
    }

    /// Confirm the pending identity (a no-op when there is none).
    pub fn promote_pending(&self) -> io::Result<()> {
        match self.load_pending()? {
            Some(identity) => self.save_identity(&identity),
            None => Ok(()),
        }
    }

    /// The identity to connect with: a pending one first (it is always newer
    /// — the band may have committed a claim whose confirmation was lost),
    /// else the confirmed one. A corrupt pending file never blocks a valid
    /// confirmed key: it is skipped with a warning (and left in place).
    pub fn connect_identity(&self) -> io::Result<Option<(EnrollmentIdentity, IdentitySource)>> {
        let pending_error = match self.load_pending() {
            Ok(Some(identity)) => return Ok(Some((identity, IdentitySource::Pending))),
            Ok(None) => None,
            Err(error) if error.kind() == io::ErrorKind::InvalidData => Some(error),
            Err(error) => return Err(error),
        };
        match (self.load_identity()?, pending_error) {
            (Some(identity), pending_error) => {
                if let Some(error) = pending_error {
                    tracing::warn!("ignoring the unreadable pending owner key: {error}");
                }
                Ok(Some((identity, IdentitySource::Confirmed)))
            }
            (None, Some(error)) => Err(error),
            (None, None) => Ok(None),
        }
    }

    pub fn save_band(&self, band: &BandRecord) -> io::Result<()> {
        let json = serde_json::to_vec_pretty(band).map_err(io::Error::other)?;
        self.write_secret(BAND, &json)
    }

    pub fn load_band(&self) -> io::Result<Option<BandRecord>> {
        match self.read(BAND)? {
            Some(bytes) => serde_json::from_slice(&bytes).map(Some).map_err(|error| {
                io::Error::new(io::ErrorKind::InvalidData, format!("{BAND}: {error}"))
            }),
            None => Ok(None),
        }
    }

    /// Remove the band record only.
    pub fn forget_band(&self) -> io::Result<()> {
        self.remove(BAND)
    }

    /// Remove every stored key, the band record and the legacy files, plus
    /// any temp file an interrupted write left behind. The Meta sign-in
    /// (`meta-session.json`) survives: signing out is a separate action
    /// (`delete_meta_session`), not implied by forgetting a band. `calibration.json`
    /// survives too: it describes the wrist.
    pub fn forget(&self) -> io::Result<()> {
        for name in [
            OWNER,
            PENDING_OWNER,
            BAND,
            LEGACY_BAND_KEY,
            LEGACY_PENDING_BAND_KEY,
        ] {
            Self::remove_path(&self.path(&format!(".{name}.tmp")))?;
            Self::remove_path(&self.path(name))?;
        }
        // Only the interrupted-write leftover for the session goes; the
        // committed file itself is kept.
        Self::remove_path(&self.path(&format!(".{META_SESSION}.tmp")))?;
        self.sync_dir()
    }

    pub fn save_meta_session(&self, session: &crate::meta::MetaSession) -> io::Result<()> {
        let json = serde_json::to_vec(session).map_err(io::Error::other)?;
        self.write_secret(META_SESSION, &json)
    }

    pub fn load_meta_session(&self) -> io::Result<Option<crate::meta::MetaSession>> {
        match self.read(META_SESSION)? {
            Some(bytes) => serde_json::from_slice(&bytes).map(Some).map_err(|error| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("{META_SESSION}: {error}"),
                )
            }),
            None => Ok(None),
        }
    }

    pub fn delete_meta_session(&self) -> io::Result<()> {
        self.remove(META_SESSION)
    }

    /// What was taught on the band at `address` for `hand` (the default when nothing was).
    /// Each field is read on its own: one that can't be read is skipped (with a warning) and
    /// the rest kept. A file that can't be read is an `InvalidData` error.
    pub fn load_calibration(&self, address: &str, hand: Hand) -> io::Result<HandCalibration> {
        let file = self.calibration_file()?;
        let entry = file.get("bands").and_then(|bands| bands.get(address)).and_then(|hands| hands.get(hand_key(hand)));
        let read = |name: &str| entry.and_then(|entry| entry.get(name)).filter(|value| !value.is_null()).cloned();
        let warn = |name: &str, error: serde_json::Error| {
            tracing::warn!("{CALIBRATION}: the {} hand's {name} for {address} can't be read ({error}); skipping it", hand_key(hand));
        };
        let palm = read("palm").and_then(|value| serde_json::from_value(value).map_err(|e| warn("palm", e)).ok());
        let drags = read("drags").and_then(|value| serde_json::from_value(value).map_err(|e| warn("drags", e)).ok());
        Ok(HandCalibration { palm, drags })
    }

    /// Replace both fields for `address` and `hand` (`None`: forgotten).
    pub fn save_calibration(&self, address: &str, hand: Hand, calibration: &HandCalibration) -> io::Result<()> {
        let palm = calibration.palm.map(serde_json::to_value).transpose().map_err(io::Error::other)?;
        let drags = calibration.drags.as_ref().map(serde_json::to_value).transpose().map_err(io::Error::other)?;
        self.edit_calibration(address, hand, [("palm", palm), ("drags", drags)])
    }

    /// Replace the palm calibration only (the drags stay as they are in the file).
    pub fn save_palm(&self, address: &str, hand: Hand, palm: Option<StoredPalm>) -> io::Result<()> {
        let palm = palm.map(serde_json::to_value).transpose().map_err(io::Error::other)?;
        self.edit_calibration(address, hand, [("palm", palm)])
    }

    /// Replace the taught drags only (the palm calibration stays as it is in the file).
    pub fn save_drags(&self, address: &str, hand: Hand, drags: Option<StoredDrags>) -> io::Result<()> {
        let drags = drags.map(serde_json::to_value).transpose().map_err(io::Error::other)?;
        self.edit_calibration(address, hand, [("drags", drags)])
    }

    /// Set (`Some`) or remove (`None`) these fields of one entry; everything else in the file
    /// is written back as it was. A file that can't be read is moved aside first, to
    /// `calibration.json.bad` (or `.bad.1`, `.bad.2`, …: none is overwritten).
    fn edit_calibration<const N: usize>(&self, address: &str, hand: Hand, fields: [(&str, Option<Value>); N]) -> io::Result<()> {
        let mut file = match self.calibration_file() {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::InvalidData => {
                let aside = self.aside_name();
                tracing::warn!("{error}; moving it aside to {aside} and starting afresh");
                fs::rename(self.path(CALIBRATION), self.path(&aside))?;
                self.sync_dir()?;
                Map::new()
            }
            Err(error) => return Err(error),
        };
        file.insert("version".into(), Value::from(CALIBRATION_VERSION));
        let bands = object_at(&mut file, "bands");
        let hands = object_at(bands, address);
        let entry = object_at(hands, hand_key(hand));
        for (name, value) in fields {
            match value {
                Some(value) => entry.insert(name.to_owned(), value),
                None => entry.remove(name),
            };
        }
        if entry.is_empty() {
            hands.remove(hand_key(hand));
        }
        if hands.is_empty() {
            bands.remove(address);
        }
        let json = serde_json::to_vec_pretty(&Value::Object(file)).map_err(io::Error::other)?;
        self.write_secret(CALIBRATION, &json)
    }

    /// The first of `calibration.json.bad`, `.bad.1`, `.bad.2`, … that doesn't exist yet.
    fn aside_name(&self) -> String {
        (0..)
            .map(|i| match i {
                0 => format!("{CALIBRATION}.bad"),
                i => format!("{CALIBRATION}.bad.{i}"),
            })
            .find(|name| !self.path(name).exists())
            .expect("some name is free")
    }

    /// The whole file (checked: version 1, `bands` an object if there); empty without one.
    fn calibration_file(&self) -> io::Result<Map<String, Value>> {
        let Some(bytes) = self.read(CALIBRATION)? else { return Ok(Map::new()) };
        let invalid = |why: String| io::Error::new(io::ErrorKind::InvalidData, format!("{CALIBRATION}: {why}"));
        let file = match serde_json::from_slice(&bytes).map_err(|error| invalid(error.to_string()))? {
            Value::Object(file) => file,
            _ => return Err(invalid("not an object".into())),
        };
        match file.get("version").and_then(Value::as_u64) {
            Some(CALIBRATION_VERSION) => {}
            Some(other) => return Err(invalid(format!("version {other} (this air-gesturesd reads {CALIBRATION_VERSION})"))),
            None => return Err(invalid("no version".into())),
        }
        match file.get("bands") {
            None | Some(Value::Null | Value::Object(_)) => Ok(file),
            Some(_) => Err(invalid("\"bands\" isn't an object".into())),
        }
    }
}

/// `map[key]` as an object (a missing, null or non-object value becomes an empty one).
fn object_at<'a>(map: &'a mut Map<String, Value>, key: &str) -> &'a mut Map<String, Value> {
    let value = map.entry(key.to_owned()).or_insert_with(|| Value::Object(Map::new()));
    if !value.is_object() {
        *value = Value::Object(Map::new());
    }
    value.as_object_mut().expect("an object: made one just above")
}
