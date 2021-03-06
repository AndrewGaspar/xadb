use std::{
    env::VarError,
    error::Error,
    io::{self, Stderr},
    time::Duration,
};

use cache::Cache;
use clap::Parser;
use cli::{Args, Command};
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use device_select::DeviceSelectApp;
use tui::{backend::CrosstermBackend, Terminal};

mod battery;
mod cache;
mod cli;
mod init_shell;
mod commands {
    pub(crate) mod adb;
    pub(crate) mod fastboot;
}
mod device_select;
mod devices;

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
        Command::List => {
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
        Command::InitShell { shell } => Ok(init_shell::init_shell(&shell)?),
        Command::Select => match std::env::var("XADB_INIT_SHELL") {
            Ok(shell) => {
                match shell.as_str() {
                    "bash" | "zsh" => (),
                    _ => {
                        panic!("Shell {shell} not supported");
                    }
                }

                let var = std::env::var("XADB_TEMP_FILE").expect("XADB_TEMP_FILE not set!");
                tokio::fs::write(
                    var,
                    format!(
                        r#"
export ANDROID_SERIAL=$({} list)
                "#,
                        std::env::current_exe().unwrap().to_str().unwrap(),
                    ),
                )
                .await?;
                Ok(())
            }
            Err(_) => {
                eprintln!(
                    r#"This shell has not be initialized. Place the following in your .bashrc:
eval "$(xadb init-shell bash)"
                    "#
                );
                std::process::exit(1);
            }
        },
        Command::Battery => {
            let level = battery::battery().await?;
            println!("{level}");
            Ok(())
        }
    }
}
