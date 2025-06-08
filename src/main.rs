extern crate exif;
use clap::Parser;
use std::process::Command;

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

fn main() -> Result<(), Error> {
    let notifier = BackupNotifier::new(
        "__CLIENT__",
        "__TOKEN__",
    );

    notifier.notify("in progress")?;

    // Command::new("mkdir -p /home/user/dev/rust/phone-media-backup-rs/sshmount && sshfs -p 2222 192.168.1.10:/sdcard/DCIM/Camera /home/user/dev/rust/phone-media-backup-rs/sshmount")
    //     .spawn();
    let args = config::Args::parse();

    let config_path = format!(
        "/home/user/dev/rust/phone-media-backup-rs/config/{}",
        args.config
    );
    let cfg: config::MyConfig =
        confy::load_path(&config_path).expect("Failed to load configuration");

    dbg!(&cfg);
    println!("{}", cfg.source.ip);

    // let sess = ssh::get_session(&cfg);
    let sess = ssh::Ssh::new(&cfg);

    // #################################################
    // get start date and build command
    let cmd = "cd ".to_owned() + &cfg.source.path + " && cat marker";
    let marker = sess.execute_command(cmd);

    let cmd = "cd ".to_owned() + &cfg.source.path + " && find . -type f -newermt " + &marker;
    let list_of_files = sess.execute_command(cmd);

    // #################################################
    // get list of files to copy

    let file_names: Vec<&str> = list_of_files.split('\n').collect();

    // #################################################
    // save files
    let latest_date = file::save_files(&cfg, file_names);
    let new_marker = latest_date.format("%Y-%m-%d").to_string();
    println!("Latest date {}", &new_marker);
    let output = Command::new("umount")
        .arg(&cfg.general.mountpoint)
        .output()
        .expect("Failed to execute command");

    if !output.status.success() {
        eprintln!(
            "Command executed with error: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    } else {
        println!(
            "Command executed successfully: {}",
            String::from_utf8_lossy(&output.stdout)
        );
    }
    // #################################################
    // set marker to latest date
    println!("Set marker");
    let cmd = "cd ".to_owned() + &cfg.source.path + " && echo " + new_marker.as_str() + "> marker";
    sess.execute_command(cmd);
    println!("marker set!");
    // println!("Total size: {}", &totalsize);
    // println!("Total size: {}", totalsize / 1024 / 1024);

    notifier.notify("success")?;

    let cmd = "cd ".to_owned()
        + &cfg.source.path
        + " && find . -type f -mtime +"
        + &cfg.source.clean.older_than_days
    // TODO: replace <file> with <rm -f> and protect this part with a cli flag to avoid accidental
    // deletion
        + " -exec file {} \\;";
    sess.execute_command(cmd);
    Ok(())
}
