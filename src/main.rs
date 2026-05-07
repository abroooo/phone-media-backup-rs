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

    let notifier = BackupNotifier::new(
        "__CLIENT__",
        "__TOKEN__",
    );
fn run_backup(cfg: &config::MyConfig, sess: &ssh::Ssh) -> anyhow::Result<()> {

    notifier.notify("in progress")?;

    let cmd = "cd ".to_owned() + &cfg.source.path + " && cat marker";
    let marker = sess.execute_command(&cmd)?;

    if !is_valid_date_format(&marker) {
        anyhow::bail!("Marker file has invalid date format. Modify or delete the marker file on the phone to fix this issue. Expected format: YYYY-MM-DD");
    }
    let list_of_files = if marker.is_empty() {
        info!("Marker file is empty, setting it to 1970-01-01");
        let cmd = "cd ".to_owned() + &cfg.source.path + " && find . -type f -newermt 1970-01-01";
        sess.execute_command(&cmd)
    } else {
        debug!("Marker file exists with date: {}", &marker);
        let cmd = "cd ".to_owned() + &cfg.source.path + " && find . -type f -newermt " + &marker;
        sess.execute_command(&cmd)
    }?;

    let file_names: Vec<&str> = list_of_files
        .split('\n')
        .filter(|s| !s.is_empty())
        .collect();

    let latest_date = file::save_files(cfg, file_names)?;
    let new_marker = latest_date.format("%Y-%m-%d").to_string();
    debug!("Latest date {}", &new_marker);

    info!("Set marker");
    let cmd = "cd ".to_owned() + &cfg.source.path + " && echo " + new_marker.as_str() + "> marker";
    sess.execute_command(&cmd)?;
    info!("marker set!");

    notifier.notify("success")?;
    Ok(())
}

fn run_clean(cfg: &config::MyConfig, sess: &ssh::Ssh, dry_run: bool) -> anyhow::Result<()> {
    let mtime_arg = cfg.source.clean.older_than_days.to_string();
    let action = if dry_run {
        "-print"
    } else {
        "-exec rm -f {} \\;"
    };
    let mode = if dry_run { "dry-run list" } else { "delete" };
    info!(
        "cleanup ({mode}): files older than {} days under {}",
        mtime_arg, &cfg.source.path
    );
    let cmd = "cd ".to_owned()
        + &cfg.source.path
        + " && find . -type f -mtime +"
        + &mtime_arg
        + " "
        + action;
    sess.execute_command(&cmd)?;
    Ok(())
}

fn run_verify(cfg: &config::MyConfig, sess: &ssh::Ssh) -> anyhow::Result<()> {
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

fn main() -> anyhow::Result<()> {
    let indicatif_layer = IndicatifLayer::new();
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_writer(indicatif_layer.get_stderr_writer()))
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(indicatif_layer)
        .init();
    let args = config::Args::parse();

    let cfg: config::MyConfig = confy::load_path(std::path::PathBuf::from(&args.config))
        .expect("Failed to load configuration");

    debug!("{:?}", &cfg);
    debug!("{}", cfg.source.ip);

    let sess = ssh::Ssh::new(&cfg)?;
    let _mount = ssh::Mount::new(&cfg)?;

    let cmd = args
        .command
        .unwrap_or_else(|| config::Commands::Backup(config::BackupArgs::default()));
    match cmd {
        config::Commands::Backup(b) => {
            run_save(&cfg, &sess)?;
            if b.clean {
                run_clean(&cfg, &sess, args.dry_run)?;
            }
        }
        config::Commands::Verify => run_verify(&cfg, &sess)?,
        config::Commands::Clean => run_clean(&cfg, &sess, args.dry_run)?,
    }

    Ok(())
}
fn is_valid_date_format(date_str: &str) -> bool {
    // Compile the regex pattern
    let re = regex::Regex::new(r"^\d{4}-\d{2}-\d{2}\s*$").unwrap();

    // Check if the string matches the pattern
    re.is_match(date_str)
}
