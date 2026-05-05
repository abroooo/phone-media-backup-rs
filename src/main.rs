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
    let marker = sess.execute_command(cmd)?;

    let cmd = "cd ".to_owned() + &cfg.source.path + " && find . -type f -newermt " + &marker;
    let list_of_files = sess.execute_command(cmd);

    // #################################################
    // get list of files to copy

    let binding = list_of_files?;
    let file_names: Vec<&str> = binding.split('\n').collect();

    // #################################################
    // save files
    let latest_date = file::save_files(cfg, file_names)?;
    let new_marker = latest_date.format("%Y-%m-%d").to_string();
    println!("Latest date {}", &new_marker);
    // #################################################
    // set marker to latest date
    println!("Set marker");
    let cmd = "cd ".to_owned() + &cfg.source.path + " && echo " + new_marker.as_str() + "> marker";
    sess.execute_command(cmd)?;
    println!("marker set!");

    notifier.notify("success")?;

    let cmd = "cd ".to_owned()
        + &cfg.source.path
        + " && find . -type f -mtime +"
        + &cfg.source.clean.older_than_days
    // TODO: replace <file> with <rm -f> and protect this part with a cli flag to avoid accidental
    // deletion
        + " -exec file {} \\;";
    sess.execute_command(cmd)?;
    Ok(())
}

fn run_verify(cfg: &config::MyConfig, sess: &ssh::Ssh) -> anyhow::Result<(), anyhow::Error> {
    // List ALL files on the phone (not just newer than marker)
    let cmd = "cd ".to_owned() + &cfg.source.path + " && find . -type f";
    let list_of_files = sess.execute_command(cmd);

    let binding = list_of_files?;
    let file_names: Vec<&str> = binding.split('\n').filter(|s| !s.is_empty()).collect();

    println!("Found {} files to verify", file_names.len());
    let missing = file::verify_files(cfg, file_names);

    if missing.is_empty() {
        println!("All files verified successfully!");
    } else {
        println!("\n{} files NOT backed up:", missing.len());
        for f in &missing {
            println!("  {}", f);
        }
    }
    Ok(())
}

fn main() -> anyhow::Result<(), anyhow::Error> {
    let args = config::Args::parse();

    let config_path = format!(
        "/home/user/dev/rust/phone-media-backup-rs/config/{}",
        args.config
    );
    let cfg: config::MyConfig =
        confy::load_path(&config_path).expect("Failed to load configuration");

    dbg!(&cfg);
    println!("{}", cfg.source.ip);

    let _mount = ssh::Mount::new(&cfg)?;
    let sess = ssh::Ssh::new(&cfg)?;

    match args.command {
        None | Some(config::Commands::Backup) => run_backup(&cfg, &sess)?,
        Some(config::Commands::Verify) => run_verify(&cfg, &sess)?,
    }

    Ok(())
}
