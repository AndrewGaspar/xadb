use std::process::Stdio;

use async_stream::try_stream;
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
};
use tokio_stream::{Stream, StreamExt};
use tokio_util::codec::FramedRead;

use crate::devices::AdbDevice;

mod logcat;

fn get_adb() -> Command {
    tokio::process::Command::new("adb")
}

pub fn shell(command: &str) -> impl Stream<Item = tokio::io::Result<String>> {
    let adb = get_adb()
        .arg("shell")
        .args(shell_words::split(command).unwrap().as_slice())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let stdout = BufReader::new(adb.stdout.unwrap());
    let mut lines = stdout.lines();

    try_stream! {
        loop {
            match lines.next_line().await? {
                Some(line) => yield line,
                None => break,
            }
        }
    }
}

pub async fn devices() -> Vec<Result<AdbDevice, crate::devices::Error>> {
    track_devices().next().await.unwrap().unwrap()
}

pub fn track_devices() -> impl Stream<
    Item = Result<
        Vec<Result<AdbDevice, crate::devices::Error>>,
        crate::devices::TrackDevicesDecodeError,
    >,
> {
    let track_devices = get_adb()
        .args(shell_words::split("track-devices -l").unwrap().as_slice())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let device_state_stream = FramedRead::new(
        BufReader::new(track_devices.stdout.unwrap()),
        crate::devices::TrackDevicesDecoder::new(),
    );

    device_state_stream
}

pub use logcat::*;
