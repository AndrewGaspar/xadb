use std::{collections::HashMap, path::PathBuf, str::FromStr};

use fd_lock::RwLock;
use home::home_dir;
use quick_error::quick_error;
use serde::{Deserialize, Serialize};
use tokio::{
    fs::OpenOptions,
    io::{AsyncWriteExt, BufWriter},
};

use crate::devices::AdbDeviceProperties;

fn xadb_dir() -> PathBuf {
    if let Ok(xadb_dir) = std::env::var("XADB_DIR") {
        PathBuf::from_str(&xadb_dir).unwrap()
    } else {
        home_dir()
            .unwrap_or_else(|| PathBuf::from_str("/").unwrap())
            .join(".xadb")
    }
}

fn cache_location() -> PathBuf {
    xadb_dir().join("cache.json")
}

quick_error! {
    #[derive(Debug)]
    pub enum Error {
        Parse(err: serde_json::Error) {
            from()
        }
        Io(err: std::io::Error) {
            from()
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct Cache {
    pub version: String,
    pub devices: HashMap<String, AdbDeviceProperties>,
}

pub type Result<T> = std::result::Result<T, Error>;

impl Cache {
    pub async fn clear() -> Result<()> {
        tokio::fs::remove_file(cache_location()).await?;
        Ok(())
    }

    pub async fn load_from_disk() -> Result<Cache> {
        match tokio::fs::read_to_string(cache_location()).await {
            Ok(contents) if contents.is_empty() => Ok(Cache {
                version: clap::crate_version!().to_string(),
                devices: Default::default(),
            }),
            Ok(contents) => Ok(serde_json::from_str(&contents)?),
            Err(_) => Ok(Cache {
                version: clap::crate_version!().to_string(),
                devices: Default::default(),
            }),
        }
    }

    pub fn save_device(&mut self, serial: &str, properties: &AdbDeviceProperties) {
        self.devices
            .entry(serial.to_owned())
            .and_modify(|e| {
                if let Some(live) = &properties.live {
                    e.live = Some(live.clone());
                }

                e.connection_state = properties.connection_state.clone();
                e.devpath = properties.devpath.clone();
            })
            .or_insert_with(|| properties.clone());
    }

    pub fn remove_device(&mut self, serial: &str) {
        self.devices.remove(serial);
    }

    pub async fn persist(&self) -> Result<()> {
        tokio::fs::create_dir_all(xadb_dir()).await?;

        let mut cache_file = RwLock::new(
            OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(false)
                .open(cache_location())
                .await?,
        );

        let mut cache_file = cache_file.try_write()?;
        cache_file.set_len(0).await?;

        let mut writer = BufWriter::new(&mut *cache_file);
        writer
            .write(serde_json::to_string(&self).unwrap().as_bytes())
            .await?;

        writer.flush().await?;

        Ok(())
    }
}
