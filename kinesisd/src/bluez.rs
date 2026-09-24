//! The real transport: BlueZ via `bluer`.
//!
//! The band advertises a name starting "Meta Band". Its GATT service `FEB8`
//! holds characteristic `2d41da7c-…` whose value is the input channel's L2CAP
//! PSM as little-endian u16 (observed `ff 00` = 255). Reading it requires an
//! encrypted link, so the first read makes BlueZ pair — during `pair` our
//! scoped agent accepts that one request. The input channel is an LE
//! credit-based L2CAP stream on that PSM (kinesis `BandConnection.swift`).

use std::collections::HashMap;
use std::time::Duration;

use bluer::agent::{Agent, AgentHandle, ReqError, ReqResult};
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
    pub async fn new() -> Result<Self, String> {
        let session = Session::new().await.map_err(describe)?;
        let adapter = session.default_adapter().await.map_err(describe)?;
        adapter.set_powered(true).await.map_err(describe)?;
        Ok(Self { session, adapter })
    }

    async fn start_le_discovery(
        &self,
    ) -> Result<impl futures::Stream<Item = AdapterEvent> + use<>, String> {
        let filter = DiscoveryFilter {
            transport: DiscoveryTransport::Le,
            ..Default::default()
        };
        self.adapter
            .set_discovery_filter(filter)
            .await
            .map_err(describe)?;
        self.adapter.discover_devices().await.map_err(describe)
    }

    /// Make sure BlueZ has a device object for `target`, discovering it if needed.
    async fn known_device(&self, target: Address) -> Result<Device, String> {
        let known = self.adapter.device_addresses().await.map_err(describe)?;
        if !known.contains(&target) {
            let events = self.start_le_discovery().await?;
            let mut events = std::pin::pin!(events);
            let seen = tokio::time::timeout(REDISCOVER, async {
                while let Some(event) = events.next().await {
                    if matches!(event, AdapterEvent::DeviceAdded(address) if address == target) {
                        return true;
                    }
                }
                false
            })
            .await
            .unwrap_or(false);
            // Dropping the stream stops discovery before we connect.
            if !seen {
                return Err(
                    "the band isn't advertising (is it nearby and awake, or in pairing mode?)"
                        .into(),
                );
            }
        }
        self.adapter.device(target).map_err(describe)
    }
}

async fn read_psm(device: &Device) -> Result<u16, String> {
    for service in device.services().await.map_err(describe)? {
        if service.uuid().await.map_err(describe)? != BAND_SERVICE {
            continue;
        }
        for characteristic in service.characteristics().await.map_err(describe)? {
            if characteristic.uuid().await.map_err(describe)? != PSM_CHARACTERISTIC {
                continue;
            }
            let value = characteristic.read().await.map_err(|error| {
                format!(
                    "reading the band's input channel failed ({error}); if the band was factory reset, run `kinesisctl pair` again"
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
        let events = self.start_le_discovery().await?;
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
                        if let Ok(Some(name)) = device.name().await
                            && is_band_name(&name)
                        {
                            let rssi = device.rssi().await.ok().flatten();
                            found.insert(address, Found { address: address.to_string(), name, rssi });
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

    async fn prepare_pairing(&self, address: &str) -> Result<AgentHandle, String> {
        let target = parse(address)?;
        // A factory reset gives the band a new identity key; a stale bond would make pairing fail.
        if let Ok(device) = self.adapter.device(target)
            && device.is_paired().await.unwrap_or(false)
        {
            self.adapter.remove_device(target).await.map_err(describe)?;
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
        self.session.register_agent(agent).await.map_err(describe)
    }

    async fn open(&self, address: &str) -> Result<Stream, String> {
        let target = parse(address)?;
        let device = self.known_device(target).await?;
        if !device.is_connected().await.map_err(describe)? {
            tokio::time::timeout(Duration::from_secs(20), device.connect())
                .await
                .map_err(|_| {
                    "the band didn't answer the connection (is it nearby and awake?)".to_string()
                })?
                .map_err(describe)?;
        }
        // The first read may wait on Bluetooth pairing.
        let psm = tokio::time::timeout(Duration::from_secs(45), read_psm(&device))
            .await
            .map_err(|_| "the band didn't open its input channel in time".to_string())??;
        let kind = device.address_type().await.map_err(describe)?;
        tokio::time::timeout(
            Duration::from_secs(20),
            Stream::connect(SocketAddr::new(target, kind, psm)),
        )
        .await
        .map_err(|_| "opening the band's input channel timed out".to_string())?
        .map_err(|error| format!("opening the band's input channel failed: {error}"))
    }

    async fn forget(&self, address: &str) -> Result<(), String> {
        match self.adapter.remove_device(parse(address)?).await {
            Ok(()) => Ok(()),
            Err(error) if is_missing(&error) => Ok(()),
            Err(error) => Err(describe(error)),
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
