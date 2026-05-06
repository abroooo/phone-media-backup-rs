#![warn(clippy::all, clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]

use clap::Parser;

mod config;
mod file;
mod helpers;
mod ssh;

use reqwest::blocking::Client;
use reqwest::Error;
use serde_json::json;
use tracing::{debug, info};
use tracing_indicatif::IndicatifLayer;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

struct BackupNotifier {
    client: Client,
    url: String,
    token: String,
}

impl BackupNotifier {
    fn new(url: &str, token: &str) -> Self {
        BackupNotifier {
            client: Client::new(),
            url: url.to_string(),
            token: token.to_string(),
        }
    }

    fn notify(&self, state: &str) -> Result<(), Error> {
        let payload = json!({ "state": state });

        self.client
            .post(&self.url)
            .bearer_auth(&self.token)
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()?
            .error_for_status()?;
        Ok(())
    }
}

fn run_backup(cfg: &config::MyConfig, sess: &ssh::Ssh) -> anyhow::Result<(), anyhow::Error> {
    let notifier = BackupNotifier::new(
        "__CLIENT__",
        "__TOKEN__",
    );

    notifier.notify("in progress")?;

    // #################################################
    // get start date and build command
    let cmd = "cd ".to_owned() + &cfg.source.path + " && cat marker";
    let marker = sess.execute_command(&cmd)?;

    let list_of_files = if marker.is_empty() {
        info!("Marker file is empty, setting it to 1970-01-01");
        let cmd = "cd ".to_owned() + &cfg.source.path + " && find . -type f -newermt 1970-01-01";
        sess.execute_command(&cmd)
    } else {
        debug!("Marker file exists with date: {}", &marker);
        let cmd = "cd ".to_owned() + &cfg.source.path + " && find . -type f -newermt " + &marker;
        sess.execute_command(&cmd)
    }?;

    // #################################################
    // get list of files to copy
    let file_names: Vec<&str> = list_of_files
        .split('\n')
        .filter(|s| !s.is_empty())
        .collect();

    // #################################################
    // save files
    let latest_date = file::save_files(cfg, file_names)?;
    let new_marker = latest_date.format("%Y-%m-%d").to_string();
    debug!("Latest date {}", &new_marker);
    // #################################################
    // set marker to latest date
    info!("Set marker");
    let cmd = "cd ".to_owned() + &cfg.source.path + " && echo " + new_marker.as_str() + "> marker";
    sess.execute_command(&cmd)?;
    info!("marker set!");

    notifier.notify("success")?;

    let cmd = "cd ".to_owned()
        + &cfg.source.path
        + " && find . -type f -mtime +"
        + &cfg.source.clean.older_than_days.to_string()
    // TODO: replace <file> with <rm -f> and protect this part with a cli flag to avoid accidental
    // deletion
        + " -exec file {} \\;";
    sess.execute_command(&cmd)?;
    Ok(())
}

fn run_verify(cfg: &config::MyConfig, sess: &ssh::Ssh) -> anyhow::Result<(), anyhow::Error> {
    // List ALL files on the phone (not just newer than marker)
    let cmd = "cd ".to_owned() + &cfg.source.path + " && find . -type f";
    let list_of_files = sess.execute_command(&cmd)?;

    let file_names: Vec<&str> = list_of_files
        .split('\n')
        .filter(|s| !s.is_empty())
        .collect();

    info!("Found {} files to verify", file_names.len());
    let missing = file::verify_files(cfg, &file_names);

    if missing.is_empty() {
        info!("All files verified successfully!");
    } else {
        info!("\n{} files NOT backed up:", missing.len());
        for f in &missing {
            info!("  {f}");
        }
    }
    Ok(())
}

fn main() -> anyhow::Result<(), anyhow::Error> {
    let indicatif_layer = IndicatifLayer::new();
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_writer(indicatif_layer.get_stderr_writer()))
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(indicatif_layer)
        .init();
    let args = config::Args::parse();

    let config_path = format!(
        "/home/user/dev/rust/phone-media-backup-rs/config/{}",
        args.config
    );
    let cfg: config::MyConfig =
        confy::load_path(&config_path).expect("Failed to load configuration");

    debug!("{:?}", &cfg);
    debug!("{}", cfg.source.ip);

    let _mount = ssh::Mount::new(&cfg)?;
    let sess = ssh::Ssh::new(&cfg)?;

    match args.command {
        None | Some(config::Commands::Backup) => run_backup(&cfg, &sess)?,
        Some(config::Commands::Verify) => run_verify(&cfg, &sess)?,
    }

    Ok(())
}
