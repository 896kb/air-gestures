//! Owner keys, the paired band and (Task 8) the Meta session, under
//! `$XDG_STATE_HOME/air-gestures` (else `~/.local/state/air-gestures`): a 0700 directory
//! of 0600 files, each written atomically and fsynced. Nothing here is logged.

use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use band_core::identity::EnrollmentIdentity;
use serde::{Deserialize, Serialize};

const OWNER: &str = "owner.key";
const PENDING_OWNER: &str = "owner.pending.key";
const BAND: &str = "band.json";
pub(crate) const META_SESSION: &str = "meta-session.json";
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
    /// (`delete_meta_session`), not implied by forgetting a band.
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
}
