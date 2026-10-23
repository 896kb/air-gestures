//! The real transport: BlueZ via `bluer`.
//!
//! The band advertises a name starting "Meta Band". Its GATT service `FEB8`
//! holds characteristic `2d41da7c-…` whose value is the input channel's L2CAP
//! PSM as little-endian u16 (observed `ff 00` = 255). Reading it requires an
//! encrypted link, so the first read makes BlueZ pair — during `pair` our
//! scoped agent accepts that one request. The input channel is an LE
//! credit-based L2CAP stream on that PSM.

use std::collections::HashMap;
use std::time::Duration;

use band_core::ceremony::FACTORY_RESET_HINT;
use bluer::agent::{Agent, AgentHandle, ReqError, ReqResult};
use bluer::gatt::remote::Service;
use bluer::l2cap::{SocketAddr, Stream};
use bluer::{
    Adapter, AdapterEvent, Address, Device, DiscoveryFilter, DiscoveryTransport, ErrorKind,
    Session, Uuid,
};
use futures::StreamExt;

use crate::transport::{Found, Transport};

const BAND_SERVICE: Uuid = Uuid::from_u128(0x0000feb8_0000_1000_8000_00805f9b34fb);
const PSM_CHARACTERISTIC: Uuid = Uuid::from_u128(0x2d41da7c_82b6_42aa_b34e_e2e01df8cc1a);
/// How long `open` listens for a band BlueZ has forgotten (e.g. right after
/// `prepare_pairing` removed a stale bond).
const REDISCOVER: Duration = Duration::from_secs(15);
const CONNECT: Duration = Duration::from_secs(30);
/// The band talks to one host at a time; a phone running air-gestures-android
/// (or the Meta AI app) keeps it from answering.
const BUSY: &str = "the band didn't answer the connection (is it nearby and awake, and not connected to your phone?)";
/// GATT service discovery after connecting.
const SERVICES: Duration = Duration::from_secs(30);
/// The PSM read, which may include Bluetooth pairing.
const PSM_READ: Duration = Duration::from_secs(45);

fn describe(error: bluer::Error) -> String {
    error.to_string()
}

fn parse(address: &str) -> Result<Address, String> {
    address
        .parse()
        .map_err(|_| format!("not a Bluetooth address: {address}"))
}

fn is_band_name(name: &str) -> bool {
    name.to_lowercase().starts_with("meta band")
}

/// "No such device": BlueZ's `DoesNotExist`, or D-Bus's unknown object.
fn is_missing(error: &bluer::Error) -> bool {
    matches!(error.kind, ErrorKind::DoesNotExist | ErrorKind::NotFound)
}

pub struct BluezTransport {
    session: Session,
    adapter: Adapter,
}

impl BluezTransport {
    /// Opens the BlueZ session and default adapter. Doesn't power the adapter
    /// on: while Bluetooth is off, `scan` and `open` say so and the daemon keeps retrying.
    pub async fn new() -> Result<Self, String> {
        let session = Session::new().await.map_err(describe)?;
        let adapter = session.default_adapter().await.map_err(describe)?;
        Ok(Self { session, adapter })
    }

    async fn ensure_powered(&self) -> Result<(), String> {
        match self.adapter.is_powered().await {
            Ok(true) => Ok(()),
            Ok(false) => Err("Bluetooth is off".into()),
            Err(error) => Err(format!("Bluetooth is unavailable: {error}")),
        }
    }

    async fn set_le_filter(&self) -> Result<(), String> {
        let filter = DiscoveryFilter {
            transport: DiscoveryTransport::Le,
            ..Default::default()
        };
        self.adapter
            .set_discovery_filter(filter)
            .await
            .map_err(describe)
    }

    /// Whether the device object at `path_address` is the band at `target`:
    /// either its object path or its Address property (the identity address
    /// once a band with a resolvable private address is bonded) matches.
    async fn is_target(&self, path_address: Address, target: Address) -> bool {
        if path_address == target {
            return true;
        }
        match self.adapter.device(path_address) {
            Ok(device) => device.remote_address().await.ok() == Some(target),
            Err(_) => false,
        }
    }

    /// The device object BlueZ already has for `target`, if any.
    async fn existing_device(&self, target: Address) -> Result<Option<Device>, String> {
        let known = self.adapter.device_addresses().await.map_err(describe)?;
        if known.contains(&target) {
            return self.adapter.device(target).map(Some).map_err(describe);
        }
        for path_address in known {
            if self.is_target(path_address, target).await {
                return self
                    .adapter
                    .device(path_address)
                    .map(Some)
                    .map_err(describe);
            }
        }
        Ok(None)
    }

    /// The address a band advertising `name` is heard at now. BlueZ also lists
    /// cached devices of that name (e.g. an old private address the band no
    /// longer uses), so like `scan` only one with an RSSI in this scan counts.
    async fn rediscover_by_name(&self, name: &str, removed: Address) -> Result<Address, String> {
        self.set_le_filter().await?;
        let events = self
            .adapter
            .discover_devices_with_changes()
            .await
            .map_err(describe)?;
        let mut events = std::pin::pin!(events);
        let seen = tokio::time::timeout(REDISCOVER, async {
            while let Some(event) = events.next().await {
                if let AdapterEvent::DeviceAdded(path_address) = event
                    && path_address != removed
                    && let Ok(device) = self.adapter.device(path_address)
                    && matches!(device.rssi().await, Ok(Some(_)))
                    && device.name().await.ok().flatten().as_deref() == Some(name)
                {
                    return Some(path_address);
                }
            }
            None
        })
        .await
        .ok()
        .flatten();
        seen.ok_or_else(|| {
            format!("{name} didn't show up again after its old pairing was removed (is it still in pairing mode?)")
        })
    }

    /// Make sure BlueZ has a device object for `target`, discovering it if needed.
    async fn known_device(&self, target: Address) -> Result<Device, String> {
        if let Some(device) = self.existing_device(target).await? {
            return Ok(device);
        }
        self.set_le_filter().await?;
        let events = self.adapter.discover_devices().await.map_err(describe)?;
        let mut events = std::pin::pin!(events);
        let seen = tokio::time::timeout(REDISCOVER, async {
            while let Some(event) = events.next().await {
                if let AdapterEvent::DeviceAdded(path_address) = event
                    && self.is_target(path_address, target).await
                {
                    return Some(path_address);
                }
            }
            None
        })
        .await
        .ok()
        .flatten();
        // Dropping the stream stops discovery before we connect.
        match seen {
            Some(path_address) => self.adapter.device(path_address).map_err(describe),
            None => Err(
                "the band isn't advertising (is it nearby and awake, or in pairing mode?)".into(),
            ),
        }
    }
}

async fn read_psm(services: Vec<Service>) -> Result<u16, String> {
    for service in services {
        if service.uuid().await.map_err(describe)? != BAND_SERVICE {
            continue;
        }
        for characteristic in service.characteristics().await.map_err(describe)? {
            if characteristic.uuid().await.map_err(describe)? != PSM_CHARACTERISTIC {
                continue;
            }
            let value = characteristic.read().await.map_err(|error| {
                format!(
                    "reading the band's input channel failed ({error}); run `air-gestures forget`, factory reset the band ({FACTORY_RESET_HINT}), then `air-gestures pair`"
                )
            })?;
            return match value.as_slice() {
                [low, high] => Ok(u16::from_le_bytes([*low, *high])),
                _ => Err("unsupported band input channel".into()),
            };
        }
    }
    Err("this device has no Neural Band input service".into())
}

fn only(target: Address, device: Address) -> ReqResult<()> {
    if device == target {
        Ok(())
    } else {
        Err(ReqError::Rejected)
    }
}

impl Transport for BluezTransport {
    type Io = Stream;
    type PairingGuard = AgentHandle;

    async fn scan(&self, timeout: Duration) -> Result<Vec<Found>, String> {
        self.ensure_powered().await?;
        self.set_le_filter().await?;
        // Known devices come first, then one event per property change: only
        // a device with an RSSI was actually heard in this scan.
        let events = self
            .adapter
            .discover_devices_with_changes()
            .await
            .map_err(describe)?;
        let mut events = std::pin::pin!(events);
        let mut found: HashMap<Address, Found> = HashMap::new();
        let deadline = tokio::time::sleep(timeout);
        let mut deadline = std::pin::pin!(deadline);
        loop {
            tokio::select! {
                _ = &mut deadline => break,
                event = events.next() => match event {
                    Some(AdapterEvent::DeviceAdded(address)) => {
                        let Ok(device) = self.adapter.device(address) else { continue };
                        let Ok(Some(rssi)) = device.rssi().await else { continue };
                        if let Ok(Some(name)) = device.name().await
                            && is_band_name(&name)
                        {
                            // The identity address once bonded, else the one heard.
                            let remote = device.remote_address().await.unwrap_or(address);
                            found.insert(address, Found { address: remote.to_string(), name, rssi: Some(rssi) });
                        }
                    }
                    Some(_) => {}
                    None => break,
                },
            }
        }
        let mut found: Vec<Found> = found.into_values().collect();
        found.sort_by_key(|band| std::cmp::Reverse(band.rssi.unwrap_or(i16::MIN)));
        Ok(found)
    }

    async fn prepare_pairing(&self, address: &str) -> Result<(AgentHandle, String), String> {
        let mut target = parse(address)?;
        if let Err(error) = self.adapter.set_pairable(true).await {
            tracing::debug!("could not make the adapter pairable: {error}");
        }
        // A factory reset gives the band a new identity key; a stale bond would make pairing fail.
        // Looked up like `open`/`forget`: the object may live under a private address.
        if let Ok(Some(device)) = self.existing_device(target).await
            && device.is_paired().await.unwrap_or(false)
        {
            let name = device.name().await.ok().flatten();
            let removed = device.address();
            self.adapter
                .remove_device(device.address())
                .await
                .map_err(describe)?;
            // Without the bond's identity key a band advertising a private
            // address can't be matched to `target` any more: find it again by name.
            if let Some(name) = name {
                target = self.rediscover_by_name(&name, removed).await?;
                tracing::info!("the band now advertises as {target}");
            }
        }
        let agent = Agent {
            request_default: true,
            request_confirmation: Some(Box::new(move |request| {
                Box::pin(async move { only(target, request.device) })
            })),
            request_authorization: Some(Box::new(move |request| {
                Box::pin(async move { only(target, request.device) })
            })),
            authorize_service: Some(Box::new(move |request| {
                Box::pin(async move { only(target, request.device) })
            })),
            ..Default::default()
        };
        let agent = self.session.register_agent(agent).await.map_err(describe)?;
        Ok((agent, target.to_string()))
    }

    async fn open(&self, address: &str) -> Result<Stream, String> {
        let target = parse(address)?;
        self.ensure_powered().await?;
        let device = self.known_device(target).await?;
        if !device.is_connected().await.map_err(describe)? {
            // Scan while connecting: the band advertises a private address that
            // changes, and BlueZ only learns the current one from a scan. Without
            // it, a band that was away (e.g. on the phone) never answers the
            // stale address. Dropped (discovery stops) once connected.
            let _discovery = match self.set_le_filter().await {
                Ok(()) => self.adapter.discover_devices().await.ok(),
                Err(_) => None,
            };
            let mut attempt = tokio::time::timeout(CONNECT, device.connect()).await;
            // BlueZ sometimes aborts the first LE connect right after discovery stops.
            if matches!(&attempt, Ok(Err(error)) if error.to_string().contains("abort-by-local")) {
                tracing::info!("the first connection attempt was aborted; retrying once");
                tokio::time::sleep(Duration::from_secs(1)).await;
                attempt = tokio::time::timeout(CONNECT, device.connect()).await;
            }
            match attempt {
                Err(_) => {
                    // Cancel BlueZ's pending Connect so it doesn't linger.
                    let _ = device.disconnect().await;
                    return Err(BUSY.into());
                }
                Ok(Err(error)) if error.kind == ErrorKind::AlreadyConnected => {}
                // An earlier Connect is still pending in BlueZ: wait for it instead
                // of stacking retries on top (each would fail "In Progress").
                // BlueZ reports it as InProgress, or as Failed "Operation already in progress".
                Ok(Err(error))
                    if error.kind == ErrorKind::InProgress
                        || error.message.contains("already in progress") =>
                {
                    let connected = tokio::time::timeout(CONNECT, async {
                        while !device.is_connected().await.unwrap_or(false) {
                            tokio::time::sleep(Duration::from_millis(500)).await;
                        }
                    })
                    .await
                    .is_ok();
                    if !connected {
                        let _ = device.disconnect().await;
                        return Err(BUSY.into());
                    }
                }
                Ok(Err(error)) => return Err(describe(error)),
                Ok(Ok(())) => {}
            }
        }
        // Asking for services before BlueZ has resolved them fails ("GATT services have
        // not been resolved"); disconnecting then leaves BlueZ mid-discovery and the
        // next connect fails "in progress". After the band re-pairs elsewhere (a phone)
        // its services are re-discovered and this can take several seconds.
        let services = tokio::time::timeout(SERVICES, async {
            while !device.is_services_resolved().await.unwrap_or(false) {
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
            device.services().await
        })
        .await
        .map_err(|_| "the band's services didn't appear".to_string())?
        .map_err(describe)?;
        // The first read may wait on Bluetooth pairing.
        let psm = tokio::time::timeout(PSM_READ, read_psm(services))
            .await
            .map_err(|_| "the band didn't open its input channel in time".to_string())??;
        if let Err(error) = device.set_trusted(true).await {
            tracing::debug!("could not mark the band trusted: {error}");
        }
        // Address and type as BlueZ now knows them (the identity address once bonded).
        let remote = device.remote_address().await.unwrap_or(target);
        let kind = device.address_type().await.map_err(describe)?;
        tokio::time::timeout(
            Duration::from_secs(20),
            Stream::connect(SocketAddr::new(remote, kind, psm)),
        )
        .await
        .map_err(|_| "opening the band's input channel timed out".to_string())?
        .map_err(|error| format!("opening the band's input channel failed: {error}"))
    }

    async fn release(&self, address: &str) {
        let Ok(target) = parse(address) else { return };
        if let Ok(Some(device)) = self.existing_device(target).await
            && device.is_connected().await.unwrap_or(false)
            && let Err(error) = device.disconnect().await
        {
            tracing::debug!("could not drop the band's Bluetooth link: {error}");
        }
    }

    async fn forget(&self, address: &str) -> Result<(), String> {
        let target = parse(address)?;
        // The object may live under a different (private) address than the identity we saved.
        let path_address = match self.existing_device(target).await {
            Ok(Some(device)) => device.address(),
            _ => target,
        };
        match self.adapter.remove_device(path_address).await {
            Ok(()) => Ok(()),
            Err(error) if is_missing(&error) => Ok(()),
            Err(error) => Err(describe(error)),
        }
    }

    async fn identity_address(&self, address: &str) -> String {
        let Ok(target) = parse(address) else {
            return address.to_owned();
        };
        match self.adapter.device(target) {
            Ok(device) => match device.remote_address().await {
                Ok(identity) => identity.to_string(),
                Err(_) => address.to_owned(),
            },
            Err(_) => address.to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn band_names_and_the_pairing_rule() {
        assert!(is_band_name("Meta Band 1A2B"));
        assert!(is_band_name("META BAND"));
        assert!(!is_band_name("Xiaomi Buds 6"));
        let band: Address = "00:00:5E:00:53:01".parse().unwrap();
        let other: Address = "00:00:5E:00:53:02".parse().unwrap();
        assert!(only(band, band).is_ok());
        assert!(matches!(only(band, other), Err(ReqError::Rejected)));
    }
}
