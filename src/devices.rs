use std::{num::ParseIntError, str::Utf8Error, time::Duration};

use async_stream::stream;
use bytes::Buf;
use quick_error::quick_error;
use regex::Regex;
use serde::{Deserialize, Serialize};
use tokio::time::MissedTickBehavior;
use tokio_stream::{Stream, StreamExt};

use crate::commands::{
    adb::{self, track_devices},
    fastboot,
};

#[derive(Clone, Debug)]
pub struct AdbDevice {
    pub connection_name: String,
    pub properties: AdbDeviceProperties,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AdbDeviceProperties {
    pub connection_state: String,
    pub devpath: String,
    #[serde(flatten)]
    pub live: Option<AdbDeviceLiveProperties>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AdbDeviceLiveProperties {
    pub product: String,
    pub model: String,
    pub device: String,
    pub transport_id: usize,
}

quick_error! {
    #[derive(Debug)]
    pub enum Error {
        TrackDevicesDecodeError(err: TrackDevicesDecodeError) {
            from()
        }
        Parse(line: String)
        Io(err: std::io::Error) {
            from()
        }
    }
}

quick_error! {
    #[derive(Debug)]
    pub enum TrackDevicesDecodeError {
        Utf8Error(err: Utf8Error) {
            from()
        }
        ParseIntError(err: ParseIntError) {
            from()
        }
        Io(err: std::io::Error) {
            from()
        }
    }
}

impl AdbDevice {
    pub fn parse(line: &str) -> Result<AdbDevice, Error> {
        lazy_static::lazy_static! {
            static ref RE: Regex = Regex::new(r"(?x)
            ^(?P<connection_name>[[[:word:]][[:punct:]]]+)
            \s+
            (?P<connection_state>[[:alpha:]]+)
            \s
            (?P<devpath>[[[:alnum:]]\-:]+)
            (?P<adb_expanded>\s
            product:(?P<product>.+)
            \s
            model:(?P<model>.+)
            \s
            device:(?P<device>.+)
            \s
            transport_id:(?P<transport_id>\d+))?").unwrap();
        }
        let captures = RE
            .captures(line)
            .ok_or_else(|| Error::Parse(line.to_string()))?;

        let connection_name = captures["connection_name"].to_string();
        let connection_state = captures["connection_state"].to_string();
        let devpath = captures["devpath"].to_string();

        let live = if captures.name("adb_expanded").is_some() {
            let product = captures["product"].to_string();
            let model = captures["model"].to_string();
            let device = captures["device"].to_string();
            let transport_id = usize::from_str_radix(&captures["transport_id"].to_string(), 10)
                .expect("Parsed as number, but did not convert to a number!");

            Some(AdbDeviceLiveProperties {
                product,
                model,
                device,
                transport_id,
            })
        } else {
            None
        };

        Ok(AdbDevice {
            connection_name,
            properties: AdbDeviceProperties {
                connection_state,
                devpath,
                live,
            },
        })
    }
}

pub async fn online_devices() -> Vec<Result<AdbDevice, crate::devices::Error>> {
    let adb_devices = adb::devices();
    let fastboot_devices = fastboot::devices();
    let (adb_devices, fastboot_devices) = tokio::join!(adb_devices, fastboot_devices);
    adb_devices.into_iter().chain(fastboot_devices).collect()
}

fn poll_fastboot(
    poll_rate: Duration,
) -> impl Stream<Item = Vec<Result<AdbDevice, crate::devices::Error>>> {
    let mut interval = tokio::time::interval(poll_rate);
    interval.set_missed_tick_behavior(MissedTickBehavior::Skip);

    stream! {
        loop {
            interval.tick().await;
            yield fastboot::devices().await;
        }
    }
}

pub fn query_devices_continuously(poll_rate: Duration) -> impl Stream<Item = Vec<AdbDevice>> {
    let mut fastboot_devices = Box::pin(poll_fastboot(poll_rate));
    let mut adb_devices = Box::pin(track_devices().filter_map(Result::ok));

    let mut current_fastboot = None;
    let mut current_adb = None;
    stream! {
        loop {
            tokio::select! {
                devices = fastboot_devices.next() => {
                    current_fastboot = devices;
                },
                devices = adb_devices.next() => {
                    current_adb = devices;
                }
            }

            match (current_fastboot.as_ref(), current_adb.as_ref()) {
                (Some(fastboot), Some(adb)) => {
                    yield fastboot.iter().chain(adb.iter()).filter_map(|x| match x {
                        Ok(devices) => Some(devices.clone()),
                        Err(_) => None,
                    }).collect();
                }
                (_, _) => {}
            }
        }
    }
}

pub struct TrackDevicesDecoder;

impl TrackDevicesDecoder {
    pub fn new() -> Self {
        Self
    }
}

impl tokio_util::codec::Decoder for TrackDevicesDecoder {
    type Item = Vec<Result<AdbDevice, Error>>;

    type Error = TrackDevicesDecodeError;

    fn decode(&mut self, src: &mut bytes::BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if src.len() < 4 {
            src.reserve(256);
            return Ok(None);
        }

        let len = u16::from_str_radix(std::str::from_utf8(&src[0..4])?, 16)? as usize;

        let message = std::str::from_utf8(&src[4..len + 4])?;

        let devices = message.lines().map(AdbDevice::parse).collect();

        src.advance(len + 4);

        Ok(Some(devices))
    }
}
