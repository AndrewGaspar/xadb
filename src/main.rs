use std::{
    env::VarError,
    error::Error,
    io::{self, Stderr},
    time::Duration,
};

use cache::Cache;
use clap::{Parser, Subcommand};
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use device_select::DeviceSelectApp;
use tui::{backend::CrosstermBackend, Terminal};

mod cache;
mod commands {
    pub(crate) mod adb;
    pub(crate) mod fastboot;
}
mod device_select;
mod devices;

#[derive(Parser)]
#[clap(author, version, about, long_about = None)]
struct Args {
    #[clap(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    #[clap(about = "Interactive selection for adb device")]
    Select,
    #[clap(about = "Clear xadb cache")]
    ClearCache,
    #[clap(about = "Get product for currently selected adb device")]
    CurrentProduct,
}

async fn build_and_run_app(
    terminal: &mut Terminal<CrosstermBackend<Stderr>>,
) -> Result<Option<String>, Box<dyn Error>> {
    // create app and run it
    let tick_rate = Duration::from_millis(250);
    let mut app = DeviceSelectApp::load_initial_state().await?;
    Ok(app.run(terminal, tick_rate).await?)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse();

    match args.command {
        Command::Select => {
            // setup terminal
            enable_raw_mode()?;
            let mut stderr = io::stderr();
            execute!(stderr, EnterAlternateScreen, EnableMouseCapture)?;
            let backend = CrosstermBackend::new(stderr);
            let mut terminal = Terminal::new(backend)?;

            let res = build_and_run_app(&mut terminal).await;

            // restore terminal
            disable_raw_mode()?;
            execute!(
                terminal.backend_mut(),
                LeaveAlternateScreen,
                DisableMouseCapture
            )?;
            terminal.show_cursor()?;

            match res {
                Ok(Some(serial)) => {
                    println!("{serial}");
                }
                Ok(None) => {}
                Err(err) => println!("{err:?}"),
            }

            Ok(())
        }
        Command::ClearCache => {
            let _ = Cache::clear().await;
            Ok(())
        }
        Command::CurrentProduct => {
            let cache = Cache::load_from_disk().await?;

            let serial = match std::env::var("ANDROID_SERIAL") {
                Ok(serial) => serial,
                Err(VarError::NotPresent) => {
                    std::process::exit(0);
                }
                Err(err) => {
                    eprintln!("Error: {:?}", err);
                    std::process::exit(1);
                }
            };

            if let Some(device) = cache.devices.get(&serial) {
                if let Some(live) = &device.live {
                    println!("{}", live.product);
                } else {
                    println!("{}", serial);
                }
            }
            Ok(())
        }
    }
}
