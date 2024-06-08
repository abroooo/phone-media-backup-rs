use clap::Parser;
use serde_derive::{Deserialize, Serialize};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
pub struct Args {
    #[arg(short, long, help = "Path to the configuration file.")]
    pub config: String,
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
    pub port: i32,
    pub cert: String,
    pub path: String,
    pub clean: Clean,
}
#[derive(Default, Debug, Serialize, Deserialize)]
pub struct Destination {
    pub path: String,
}
#[derive(Default, Debug, Serialize, Deserialize)]
pub struct General {
    pub mountpoint: String,
}
#[derive(Default, Debug, Serialize, Deserialize)]
pub struct Clean {
    pub older_than_days: String,
}
