//! Moving the band between devices: `air-gestures-band.json`, the owner key
//! (base64) and band record in one file that this app and the Android app both
//! write and read. Key material is never printed or put in an error message.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;

use air_gesturesd::store::{self, BandRecord, Store};
use band_core::identity::EnrollmentIdentity;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};

pub const FORMAT: &str = "air-gestures-band";
pub const VERSION: u64 = 1;
/// Where `air-gestures export` writes when no path is given.
pub const DEFAULT_FILE: &str = "air-gestures-band.json";

#[derive(Serialize, Deserialize)]
struct Bundle {
    format: String,
    version: u64,
    owner_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    band: Option<BandRecord>,
}

/// What an import file held: a checked owner key and, from a bundle, maybe the band.
pub struct Imported {
    pub identity: EnrollmentIdentity,
    pub band: Option<BandRecord>,
}

/// The bundle for the store's confirmed owner key (a pending one is refused:
/// the band may not have committed it) and band record, if any.
pub fn bundle(store: &Store) -> Result<Vec<u8>, String> {
    let identity = match store.load_identity() {
        Ok(Some(identity)) => identity,
        Ok(None) if store.has_pending() => {
            return Err(
                "the claim isn't confirmed yet: connect once (air-gestures connect) so the band confirms it, then export"
                    .into(),
            );
        }
        Ok(None) => return Err("no owner key here — claim a band with `air-gestures pair`".into()),
        Err(error) => return Err(format!("could not read the owner key: {error}")),
    };
    let band = store
        .load_band()
        .map_err(|error| format!("could not read the band record: {error}"))?;
    let bundle = Bundle {
        format: FORMAT.into(),
        version: VERSION,
        owner_key: STANDARD.encode(store::identity_bytes(&identity)),
        band,
    };
    let mut json = serde_json::to_vec_pretty(&bundle).map_err(|error| error.to_string())?;
    json.push(b'\n');
    Ok(json)
}

/// Write the bundle to `path` as a 0600 file; an existing file is only
/// overwritten with `force`.
pub fn export(store: &Store, path: &Path, force: bool) -> Result<(), String> {
    let json = bundle(store)?;
    let mut options = OpenOptions::new();
    options.write(true).mode(0o600);
    if force {
        options.create(true).truncate(true);
    } else {
        options.create_new(true);
    }
    let mut file = options.open(path).map_err(|error| match error.kind() {
        io::ErrorKind::AlreadyExists => format!(
            "{} already exists (use --force to overwrite it)",
            path.display()
        ),
        _ => format!("could not create {}: {error}", path.display()),
    })?;
    // An overwritten file keeps its old mode otherwise.
    file.set_permissions(fs::Permissions::from_mode(0o600))
        .and_then(|()| file.write_all(&json))
        .and_then(|()| file.sync_all())
        .map_err(|error| format!("could not write {}: {error}", path.display()))
}

/// Read an import file: the bundle, or (for convenience) a raw `owner.key`.
pub fn parse(bytes: &[u8]) -> Result<Imported, String> {
    match serde_json::from_slice::<serde_json::Value>(bytes) {
        Ok(serde_json::Value::Object(object)) => parse_bundle(object),
        // A raw 32-byte key can happen to be valid JSON (a lone digit, say);
        // anything of a key's length that isn't an object is read as one.
        _ if bytes.len() == 32 || bytes.len() == 97 => Ok(Imported {
            identity: store::identity_from_bytes(bytes)
                .map_err(|error| format!("that isn't a usable owner key: {error}"))?,
            band: None,
        }),
        _ => Err(format!(
            "not an air-gestures band file (expected {DEFAULT_FILE} or an owner.key)"
        )),
    }
}

fn parse_bundle(object: serde_json::Map<String, serde_json::Value>) -> Result<Imported, String> {
    if object.get("format").and_then(|f| f.as_str()) != Some(FORMAT) {
        return Err(format!(
            "not an air-gestures band file (its \"format\" isn't \"{FORMAT}\")"
        ));
    }
    match object.get("version").and_then(|v| v.as_u64()) {
        Some(VERSION) => {}
        Some(version) if version > VERSION => {
            return Err(format!(
                "this band file is version {version}, newer than this air-gestures understands ({VERSION}); update air-gestures"
            ));
        }
        _ => return Err("this band file has no usable \"version\"".into()),
    }
    let bundle: Bundle = serde_json::from_value(serde_json::Value::Object(object))
        .map_err(|error| format!("this band file is malformed: {error}"))?;
    let key = STANDARD
        .decode(bundle.owner_key.trim())
        .map_err(|_| "this band file's owner key isn't valid base64".to_string())?;
    let identity = store::identity_from_bytes(&key)
        .map_err(|error| format!("this band file's owner key isn't usable: {error}"))?;
    Ok(Imported {
        identity,
        band: bundle.band,
    })
}

/// Save the imported key as the confirmed one (dropping any pending key) and
/// the band record when the file had one; an existing record is kept otherwise.
pub fn import(store: &Store, imported: &Imported) -> Result<(), String> {
    store
        .save_identity(&imported.identity)
        .map_err(|error| format!("could not save the owner key: {error}"))?;
    if let Some(band) = &imported.band {
        // A phone may record the band's resolvable private address, which the band
        // changes every few minutes and which is useless to this computer. When this
        // computer already knows the band under the same name, keep its address (the
        // identity address BlueZ resolved when it bonded).
        let existing = store.load_band().ok().flatten();
        let band = match existing {
            Some(known) if is_private_address(&band.address) && known.name == band.name => {
                BandRecord {
                    address: known.address,
                    ..band.clone()
                }
            }
            _ => band.clone(),
        };
        store
            .save_band(&band)
            .map_err(|error| format!("could not save the band record: {error}"))?;
    }
    Ok(())
}

/// A resolvable private address: random, with the top two bits of the first octet `01`.
pub fn is_private_address(address: &str) -> bool {
    u8::from_str_radix(address.get(..2).unwrap_or(""), 16).is_ok_and(|first| first >> 6 == 0b01)
}
