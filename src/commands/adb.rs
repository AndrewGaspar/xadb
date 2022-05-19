use std::process::Stdio;

use async_stream::try_stream;
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
};
use tokio_stream::Stream;

use crate::devices::AdbDevice;

fn get_adb() -> Command {
    tokio::process::Command::new("adb")
}

pub fn devices() -> impl Stream<Item = Result<AdbDevice, crate::devices::Error>> {
    let adb = get_adb()
        .args(shell_words::split("devices -l").unwrap().as_slice())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let stdout = BufReader::new(adb.stdout.unwrap());
    let mut lines = stdout.lines();

    try_stream! {
        let first = lines.next_line().await?;
        assert_eq!(
            Some("List of devices attached"),
            first.as_ref().map(|s| s.as_str())
        );

        loop {
            match lines.next_line().await? {
                Some(empty) if empty == "" => break,
                Some(line) => yield AdbDevice::parse(&line)?,
                None => break,
            }
        }
    }
}
