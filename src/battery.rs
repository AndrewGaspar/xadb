use quick_error::quick_error;
use regex::Regex;
use tokio::pin;
use tokio_stream::StreamExt;

use crate::commands::adb;

quick_error! {
    #[derive(Debug)]
    pub enum Error {
        Io(err: std::io::Error) {
            from()
        }
        NotFound
    }
}

pub async fn battery() -> Result<i32, Error> {
    lazy_static::lazy_static! {
        static ref RE: Regex = Regex::new(r"(?x)
        ^\s\slevel:\s(?P<level>[[:xdigit:]]+)").unwrap();
    }

    let stream = adb::shell("dumpsys battery");
    pin!(stream);

    while let Some(line) = stream.next().await {
        let line = line?;
        if let Some(captures) = RE.captures(&line) {
            return Ok(i32::from_str_radix(&captures["level"], 10).unwrap());
        }
    }

    Err(Error::NotFound)
}
