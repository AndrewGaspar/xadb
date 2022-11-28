use std::{collections::HashMap, thread::current, time::Duration};

use async_stream::try_stream;
use quick_error::quick_error;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncRead, BufReader};
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
    pub properties: HashMap<String, serde_json::Value>,
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

fn split_property(property: &str) -> Option<(&str, &str)> {
    Some(property.split_at(property.find(':')?))
}

#[derive(Copy, Clone, PartialEq, PartialOrd, Eq, Ord)]
enum ParseDeviceState {
    Serial,
    ConnectionState,
    Properties,
}

struct ParseDeviceStateMachine<S>
where
    S: Stream<Item = char> + std::marker::Unpin,
{
    stream: S,
    state: ParseDeviceState,
    current_token: Option<String>,
    serial: Option<String>,
    connection_state: Option<String>,
    devpath: Option<String>,
    properties: HashMap<String, Value>,
}

impl<S> ParseDeviceStateMachine<S>
where
    S: Stream<Item = char> + std::marker::Unpin,
{
    pub fn new(stream: S) -> Self {
        ParseDeviceStateMachine {
            stream,
            state: ParseDeviceState::Serial,
            current_token: None,
            serial: None,
            connection_state: None,
            devpath: None,
            properties: Default::default(),
        }
    }

    fn flush_current_token(&mut self) {
        match self.state {
            ParseDeviceState::Serial => {
                self.serial = self.current_token.take();
            }
            ParseDeviceState::ConnectionState => {
                self.connection_state = self.current_token.take();
            }
            ParseDeviceState::Properties => {
                if let Some(token) = self.current_token.take() {
                    if let Some((key, value)) = split_property(&token) {
                        self.properties
                            .insert(key.to_owned(), serde_json::Value::from(value.to_owned()));
                    }
                }
            }
        }
    }

    pub fn parse_all<'a>(&'a mut self) -> impl 'a + Stream<Item = Result<AdbDevice, Error>> {
        try_stream! {
            while let Some(c) = self.stream.next().await {
                match c {
                    '\n' if self.state != ParseDeviceState::ConnectionState => {
                        self.flush_current_token();
                    }
                }
            }

            yield AdbDevice {
                serial: self.serial.take().unwrap(),
                properties: AdbDeviceProperties {
                    connection_state: self.connection_state.take().unwrap(),
                    devpath: self.devpath.take().unwrap(),
                    properties: self.properties.clone(),
                },
            };
        }
    }
}

pub fn parse_devices<S>(mut stream: S) -> impl Stream<Item = Result<AdbDevice, Error>>
where
    S: Stream<Item = char> + std::marker::Unpin,
{
    let state_machine = ParseDeviceStateMachine::new(stream);

    let out_stream = state_machine.parse_all();
    try_stream! {
        while let Some(device) = out_stream.next().await {
            yield Ok(device);
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

        todo!()

        // Ok(AdbDevice {
        //     serial,
        //     properties: AdbDeviceProperties {
        //         connection_state,
        //         devpath,
        //         live,
        //     },
        // })
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
