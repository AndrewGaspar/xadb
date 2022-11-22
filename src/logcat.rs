use std::{collections::HashSet, io::Stderr};

use quick_error::quick_error;
use tokio_stream::StreamExt;
use tui::{backend::CrosstermBackend, Terminal};

use crate::commands::adb::LogBuffer;

quick_error! {
    #[derive(Debug)]
    pub enum Error {
        Io(err: crate::io::Error) {
            from()
        }
        Decode(err: crate::commands::adb::LogcatDecodeError) {
            from()
        }
        DeviceSelect(err: crate::device_select::Error) {
            from()
        }
    }
}

pub struct LogcatApp {}

impl LogcatApp {
    pub async fn run(
        terminal: Option<&mut Terminal<CrosstermBackend<Stderr>>>,
    ) -> Result<(), Error> {
        let serial = match std::env::var("ANDROID_SERIAL") {
            Ok(serial) => serial,
            _ => {
                let mut device_list =
                    crate::device_select::DeviceSelectApp::load_initial_state().await?;

                match device_list
                    .run(terminal.unwrap(), std::time::Duration::from_millis(250))
                    .await?
                {
                    Some(serial) => serial,
                    None => std::process::exit(1),
                }
            }
        };

        let mut logs = crate::commands::adb::logcat(serial.as_str());

        let mut set = HashSet::new();
        while let Some(Ok(message)) = logs.next().await {
            if let LogBuffer::TextLog(buffer) = message.buffer {
                // match log {
                //     LogItem::LogMessage(message) => {
                if !set.contains(&buffer.tag) {
                    println!("{}", buffer.tag);
                    set.insert(buffer.tag);
                }
                println!("{}", buffer.message);
            }
        }

        Ok(())
    }
}
