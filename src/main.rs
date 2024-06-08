// use ssh2::Session;
use std::fs::File;
use std::io::{Cursor, Read, Seek, SeekFrom};
extern crate exif;
use indicatif::ProgressBar;

use std::env;
use std::io::BufReader;
use std::time::SystemTime;

use exif::{DateTime, In, Reader, Tag, Value};

use chrono::{NaiveDate, NaiveDateTime};
use std::process::Command;

mod config;
mod file;
mod helpers;
mod ssh;
fn save_files(cfg: &config::MyConfig, file_names: Vec<&str>) -> NaiveDateTime {
    let base_dir = &cfg.general.mountpoint;
    let mut latest_date: NaiveDateTime = Default::default();
    let mut totalsize: u64 = 0;
    let bar = ProgressBar::new(file_names.len().try_into().unwrap());
    for base_name in file_names {
        bar.inc(1);
        let path_name = base_dir.to_owned() + base_name.as_ref();
        // println!("name: {}", path_name);
        let fsize = std::path::PathBuf::from(&path_name)
            .metadata()
            .unwrap()
            .len();
        totalsize = totalsize + fsize;
        println!("filesize: {} MB", fsize / 1024 / 1024);
        let t = path_name;
        let x = helpers::get_extension_from_filename(&t);
        match x {
            Some(x) => match x {
                "jpg" | "dng" => {
                    println!("{:?}", &t);
                    let file = File::open(&t.to_string()).unwrap();
                    let exif = Reader::new()
                        .read_from_container(&mut BufReader::new(&file))
                        .unwrap();
                    if let Some(field) = exif.get_field(Tag::DateTime, In::PRIMARY) {
                        match field.value {
                            Value::Ascii(ref vec) if !vec.is_empty() => {
                                if let Ok(datetime) = DateTime::from_ascii(&vec[0]) {
                                    // println!("Year of DateTime is {}.", &datetime.year);
                                    // println!("Month of DateTime is {:02}.", &datetime.month);
                                    // println!("Day of DateTime is {}.", datetime.day);
                                    // println!("{}", &datetime.to_string());
                                    let datetime_native = NaiveDateTime::parse_from_str(
                                        &datetime.to_string(),
                                        "%Y-%m-%d %H:%M:%S",
                                    )
                                    .unwrap();
                                    if datetime_native > latest_date {
                                        latest_date = datetime_native;
                                    }
                                    file::move_file(helpers::DateLoc {
                                        year: datetime.year.to_string(),
                                        month: format!("{:02}", datetime.month),
                                        source: t.to_string(),
                                        base_name: base_name.to_string(),
                                        destination_base: cfg.destination.path.to_owned(),
                                    });
                                }
                            }
                            _ => {}
                        }
                    }
                }
                "mp4" => {
                    println!("{:?}", &t);
                    let re = regex::Regex::new(r"\d{8}").unwrap();
                    if let Some(date_match) = re.find(&t) {
                        let year: String = date_match.as_str().chars().take(4).collect();
                        let month: String = date_match.as_str().chars().skip(4).take(2).collect();
                        println!("year: {}", &year.to_string());
                        println!("month: {}", &month.to_string());
                        file::move_file(helpers::DateLoc {
                            year: year.to_string(),
                            month: month.to_string(),
                            source: t.to_string(),
                            base_name: base_name.to_string(),
                            destination_base: cfg.destination.path.to_owned(),
                        });
                    } else {
                        println!("No date found in the string.");
                    }
                }
                _ => {
                    println!("dont care");
                }
            },
            None => {
                println!("dont care {}", t);
            }
        }
        println!("_________________________________________________________________________________________");
    }
    bar.finish();
    latest_date
}

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
    let latest_date = save_files(&cfg, file_names);
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
    let marker = sess.execute_command(cmd);
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
    let res = sess.execute_command(cmd);
}
