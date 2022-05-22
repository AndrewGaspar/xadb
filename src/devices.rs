use std::time::Duration;

use async_stream::try_stream;
use futures::channel::mpsc;
use quick_error::quick_error;
use regex::Regex;
use serde::{Deserialize, Serialize};
use tokio::task::JoinHandle;
use tokio_stream::{Stream, StreamExt};

use crate::commands::{adb, fastboot};

#[derive(Clone, Debug)]
pub struct AdbDevice {
    pub serial: String,
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
        Parse(line: String)
        Io(err: std::io::Error) {
            from()
        }
    }
}

impl AdbDevice {
    pub fn parse(line: &str) -> Result<AdbDevice, Error> {
        lazy_static::lazy_static! {
            static ref RE: Regex = Regex::new(r"(?x)
            ^(?P<serial>[[:xdigit:]]+)
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

        let serial = captures["serial"].to_string();
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
            serial,
            properties: AdbDeviceProperties {
                connection_state,
                devpath,
                live,
            },
        })
    }
}

pub fn online_devices() -> impl Stream<Item = Result<AdbDevice, crate::devices::Error>> {
    let adb_devices = adb::devices();
    let fastboot_devices = fastboot::devices();
    adb_devices.chain(fastboot_devices)
}

pub fn query_devices_continuously(
    poll_rate: Duration,
) -> impl Stream<Item = Result<Vec<AdbDevice>, crate::devices::Error>> {
    try_stream! {
        loop {
            let devices = online_devices().collect().await;
            yield devices?;
            tokio::time::sleep(poll_rate).await;
        }
    }
}

pub struct DeviceServiceBuilder {
    auto_poll_rate: Option<Duration>,
    auto_cache_rate: Option<Duration>,
}

impl DeviceServiceBuilder {
    pub fn new() -> Self {
        Self {
            auto_poll_rate: None,
            auto_cache_rate: None,
        }
    }

    pub fn auto_poll(&mut self, rate: Duration) -> &mut Self {
        self.auto_poll_rate = Some(rate);
        self
    }

    pub fn auto_cache(&mut self, rate: Duration) -> &mut Self {
        self.auto_poll_rate = Some(rate);
        self
    }

    pub fn build() -> DeviceService {
        let task = tokio::spawn(device_service_loop());

        DeviceService { task }
    }
}

pub enum DeviceState {
    Device,
    Fastboot,
    Offline,
}

pub enum DeviceEventType {
    Hello,
    StateChange { old: String, new: String },
}

pub struct DeviceEvent {
    serial: String,
    event: DeviceEventType,
}

struct DeviceServiceSubscription {
    tx: mpsc::Sender<Result<DeviceEvent, Error>>,
}

pub struct DeviceService {
    task: JoinHandle<()>,
    new_subscribers_tx: mpsc::Sender<DeviceServiceSubscription>,
}

impl DeviceService {
    pub fn subscribe_device_events(&self) -> impl Stream<Item = Result<DeviceEvent, Error>> {
        let (tx, rx) = mpsc::channel(32);

        let subscription = DeviceServiceSubscription { tx };

        rx
    }
}

async fn device_service_loop(mut new_subscribers_rx: mpsc::Receiver<DeviceServiceSubscription>) {
    let cache = crate::cache::Cache::load_from_disk().await;

    let mut subscriptions = vec![];

    loop {
        while let Ok(Some(subscription)) = new_subscribers_rx.try_next() {
            subscriptions.push(subscription);
        }
    }
}
