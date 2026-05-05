use std::path::PathBuf;

use clap::{Parser, Subcommand};
use serde_derive::{Deserialize, Serialize};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
pub struct Args {
    #[arg(short, long, help = "Path to the configuration file.")]
    pub config: String,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Run the backup (default when no subcommand is given)
    Backup,
    /// Verify that all files on the phone have been backed up
    Verify,
}

#[derive(Default, Debug, Serialize, Deserialize)]
pub struct MyConfig {
    pub general: General,
    pub source: Source,
    pub destination: Destination,
}
#[derive(Default, Debug, Serialize, Deserialize)]
pub struct Source {
    pub ip: String,
    pub port: u16,
    pub cert: String,
    pub path: String,
    pub clean: Clean,
}
#[derive(Default, Debug, Serialize, Deserialize)]
pub struct Destination {
    pub path: PathBuf,
}
#[derive(Default, Debug, Serialize, Deserialize)]
pub struct General {
    pub mountpoint: String,
}
#[derive(Default, Debug, Serialize, Deserialize)]
pub struct Clean {
    pub older_than_days: String,
}
