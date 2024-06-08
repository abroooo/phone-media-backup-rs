// use ssh2::Session;
extern crate exif;
use std::process::Command;

mod config;
mod file;
mod helpers;
mod ssh;
fn main() {
    // Command::new("mkdir -p /home/user/dev/rust/phone-media-backup-rs/sshmount && sshfs -p 2222 192.168.1.10:/sdcard/DCIM/Camera /home/user/dev/rust/phone-media-backup-rs/sshmount")
    //     .spawn();
    let cfg: config::MyConfig =
        confy::load_path("/home/user/dev/rust/phone-media-backup-rs/op7p").unwrap();
    // confy::load_path("/home/user/dev/rust/phone-media-backup-rs/op7p").unwrap();
    // confy::load_path("/home/user/dev/rust/phone-media-backup-rs/config").unwrap();
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

    // // delete files older than 60 days
    // find . -type f -mtime +60 -exec rm -f {} \;
    let cmd = "cd ".to_owned()
        + &cfg.source.path
        + " && find . -type f -mtime +"
        + &cfg.source.clean.older_than_days
    // TODO: replace <file> with <rm -f> and protect this part with a cli flag to avoid accidental
    // deletion
        + " -exec file {} \\;";
    sess.execute_command(cmd);
}
