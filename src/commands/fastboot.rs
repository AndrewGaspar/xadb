use std::process::Stdio;

use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
};

use crate::devices::AdbDevice;

fn get_fastboot() -> Command {
    tokio::process::Command::new("fastboot")
}

pub async fn devices() -> Vec<Result<AdbDevice, crate::devices::Error>> {
    let adb = get_fastboot()
        .args(shell_words::split("devices -l").unwrap().as_slice())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let stdout = BufReader::new(adb.stdout.unwrap());
    let mut lines = stdout.lines();

    let mut devices = Vec::new();
    while let Ok(Some(line)) = lines.next_line().await {
        devices.push(AdbDevice::parse(&line));
    }
    devices
}
