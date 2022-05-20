use clap::{Parser, Subcommand};

#[derive(Parser)]
#[clap(author, version, about, long_about = None)]
pub struct Args {
    #[clap(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    #[clap(about = "Interactive list of adb devices")]
    List,
    #[clap(about = "Clear xadb cache")]
    ClearCache,
    #[clap(about = "Get product for currently selected adb device")]
    CurrentProduct,
    #[clap(about = "Print shell integration function")]
    InitShell { shell: String },
    #[clap(about = "Interactively select adb device to use in current shell")]
    Select,
    #[clap(about = "Get battery level for adb device")]
    Battery,
}
