//! Owner keys, the paired band and (Task 8) the Meta session, under
//! `$XDG_STATE_HOME/kinesis` (else `~/.local/state/kinesis`): a 0700 directory
//! of 0600 files, each written atomically. Nothing here is logged.

use std::fs::{self, DirBuilder, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use band_core::identity::EnrollmentIdentity;
use serde::{Deserialize, Serialize};

const OWNER: &str = "owner.key";
const BAND_KEY: &str = "band.pub";
const PENDING_OWNER: &str = "owner.pending.key";
const PENDING_BAND_KEY: &str = "band.pending.pub";
const BAND: &str = "band.json";
pub(crate) const META_SESSION: &str = "meta-session.json";

/// The paired band: its Bluetooth address and advertised name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BandRecord {
    pub address: String,
    pub name: String,
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

    /// `$XDG_STATE_HOME/kinesis`, else `~/.local/state/kinesis`.
    pub fn default_dir() -> PathBuf {
        std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| crate::config::home().join(".local/state"))
            .join("kinesis")
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    pub(crate) fn write_secret(&self, name: &str, bytes: &[u8]) -> io::Result<()> {
        let temp = self.path(&format!(".{name}.tmp"));
        {
            let mut file = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&temp)?;
            file.write_all(bytes)?;
            file.sync_all()?;
        }
        fs::rename(&temp, self.path(name))
    }

    pub(crate) fn read(&self, name: &str) -> io::Result<Option<Vec<u8>>> {
        match fs::read(self.path(name)) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    pub(crate) fn remove(&self, name: &str) -> io::Result<()> {
        match fs::remove_file(self.path(name)) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        }
    }

    fn write_identity(
        &self,
        owner: &str,
        band_key: &str,
        identity: &EnrollmentIdentity,
    ) -> io::Result<()> {
        self.write_secret(owner, &identity.private_bytes())?;
        match identity.band_key_x963() {
            Some(key) => self.write_secret(band_key, &key),
            None => self.remove(band_key),
        }
    }

    fn read_identity(&self, owner: &str, band_key: &str) -> io::Result<Option<EnrollmentIdentity>> {
        let Some(private) = self.read(owner)? else {
            return Ok(None);
        };
        let band = self.read(band_key)?;
        EnrollmentIdentity::from_bytes(&private, band.as_deref())
            .map(Some)
            .map_err(|error| {
                io::Error::new(io::ErrorKind::InvalidData, format!("{owner}: {error}"))
            })
    }

    /// Store the confirmed owner identity and drop any pending one.
    pub fn save_identity(&self, identity: &EnrollmentIdentity) -> io::Result<()> {
        self.write_identity(OWNER, BAND_KEY, identity)?;
        self.remove(PENDING_OWNER)?;
        self.remove(PENDING_BAND_KEY)
    }

    pub fn load_identity(&self) -> io::Result<Option<EnrollmentIdentity>> {
        self.read_identity(OWNER, BAND_KEY)
    }

    /// Store the identity the band may commit to before its confirmation arrives.
    pub fn save_pending(&self, identity: &EnrollmentIdentity) -> io::Result<()> {
        self.write_identity(PENDING_OWNER, PENDING_BAND_KEY, identity)
    }

    pub fn load_pending(&self) -> io::Result<Option<EnrollmentIdentity>> {
        self.read_identity(PENDING_OWNER, PENDING_BAND_KEY)
    }

    /// The identity to connect with: the confirmed one, else a pending one
    /// (the band may have committed a claim whose confirmation was lost).
    pub fn connect_identity(&self) -> io::Result<Option<EnrollmentIdentity>> {
        match self.load_identity()? {
            Some(identity) => Ok(Some(identity)),
            None => self.load_pending(),
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

    /// Remove every stored key, the band record and the Meta session.
    pub fn forget(&self) -> io::Result<()> {
        for name in [
            OWNER,
            BAND_KEY,
            PENDING_OWNER,
            PENDING_BAND_KEY,
            BAND,
            META_SESSION,
        ] {
            self.remove(name)?;
        }
        Ok(())
    }
}
