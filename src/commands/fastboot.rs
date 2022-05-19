use std::process::Stdio;

use async_stream::try_stream;
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
};
use tokio_stream::Stream;

use crate::devices::AdbDevice;

fn get_fastboot() -> Command {
    tokio::process::Command::new("fastboot")
}

pub fn devices() -> impl Stream<Item = Result<AdbDevice, crate::devices::Error>> {
    let adb = get_fastboot()
        .args(shell_words::split("devices -l").unwrap().as_slice())
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
                Some(empty) if empty == "" => break,
                Some(line) => yield AdbDevice::parse(&line)?,
                None => break,
            }
        }
    }
}
