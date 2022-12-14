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
mod fps_overlay;
mod init_shell;
mod logcat;
mod status;

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

fn is_tui(args: &Args) -> bool {
    match args.command {
        Command::List | Command::Logcat => true,
        _ => false,
    }
}

struct TuiConfiguration {
    terminal: Terminal<CrosstermBackend<Stderr>>,
}

impl TuiConfiguration {
    fn try_drop(&mut self) -> Result<(), Box<dyn Error>> {
        // restore terminal
        disable_raw_mode()?;
        execute!(
            self.terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )?;
        self.terminal.show_cursor()?;
        Ok(())
    }
}

impl Drop for TuiConfiguration {
    fn drop(&mut self) {
        let _ignored = self.try_drop();
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse();

    // for TUI commands, set up terminal
    let mut maybe_terminal = if is_tui(&args) {
        enable_raw_mode()?;
        let mut stderr = io::stderr();
        execute!(stderr, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stderr);
        Some(TuiConfiguration {
            terminal: Terminal::new(backend)?,
        })
    } else {
        None
    };

    match args.command {
        Command::List => {
            let terminal = maybe_terminal.as_mut().unwrap();

            let res = build_and_run_app(&mut terminal.terminal).await;

            // drop terminal before printing output
            std::mem::drop(maybe_terminal);

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
XADB_ANDROID_SERIAL_SELECT=$({} list)
if [ ! -z "$XADB_ANDROID_SERIAL_SELECT" ]; then
  export ANDROID_SERIAL="$XADB_ANDROID_SERIAL_SELECT"
fi
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
        Command::Logcat => {
            let terminal = maybe_terminal.as_mut().unwrap();

            let mut app = logcat::LogcatApp::new();
            app.run(&mut terminal.terminal).await?;
            Ok(())
        }
    }
}
