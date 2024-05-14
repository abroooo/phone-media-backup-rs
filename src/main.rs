use ssh2::Session;
use std::fs::File;
use std::io::{Cursor, Read, Seek, SeekFrom};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::Command;
extern crate exif;
use indicatif::ProgressBar;

use std::env;
use std::io::BufReader;
use std::time::SystemTime;

use exif::{DateTime, In, Reader, Tag, Value};

use chrono::{NaiveDate, NaiveDateTime};


mod config;
mod file;
mod helpers;

fn main() {
    // Command::new("mkdir -p /home/user/dev/rust/phone-media-backup-rs/sshmount && sshfs -p 2222 192.168.1.10:/sdcard/DCIM/Camera /home/user/dev/rust/phone-media-backup-rs/sshmount")
    //     .spawn();
    let cfg: config::MyConfig = confy::load_path("/home/user/dev/rust/phone-media-backup-rs/op7p").unwrap();
    // confy::load_path("/home/user/dev/rust/phone-media-backup-rs/config").unwrap();
    dbg!(&cfg);
    println!("{}", cfg.source.ip);

    // create ssh session
    let remote_user = "user";
    // let remote_host = "1.1.0.2";
    // let remote_user = "your_username";
    // let private_key_path = "/path/to/your/private_key";
    let private_key_path = Path::new(&cfg.source.cert);

    // mount sshfs
    let port = "2222"; // Replace with your port number
    let path = cfg.source.ip.to_string() + ":/sdcard/DCIM/Camera"; // Replace with your remote path

    let output = Command::new("sshfs")
        .arg("-p")
        .arg(&cfg.source.port.to_string())
        .arg(path)
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

    // Connect to the remote SSH server
    let tcp = TcpStream::connect(format!("{}:{}", &cfg.source.ip, &cfg.source.port)).unwrap();
    let mut sess = Session::new().unwrap();
    sess.set_tcp_stream(tcp);
    sess.handshake().unwrap();

    // Authenticate using a certificate (public key)
    sess.userauth_pubkey_file(
        remote_user,
        None,
        PathBuf::from(&cfg.source.cert).as_path(),
        None,
    )
    .unwrap();

    // Check if authentication was successful
    assert!(sess.authenticated());

    // #################################################
    // get start date and build command
    let cmd = "cd ".to_owned() + &cfg.source.path + " && cat marker";
    let mut channel = sess.channel_session().unwrap();
    channel
        .exec(&cmd)
        // .exec("ls -l /sdcard/DCIM/Camera | awk '{print $8}'")
        .unwrap();
    let mut s = String::new();
    channel.read_to_string(&mut s).unwrap();
    println!("{}", &s);
    let command = "cd ".to_owned() + &cfg.source.path + " && find . -type f -newermt " + &s;
    println!("Command to run:\n{}", &command);

    // #################################################
    // get list of files to copy

    // create a new channel since the old one cannot be reused
    let mut channel = sess.channel_session().unwrap();
    // TODO: handle unwrap
    channel.exec(&command).unwrap();
    let mut s = String::new();
    // TODO: handle unwrap
    channel.read_to_string(&mut s).unwrap();
    let file_names: Vec<&str> = s.split('\n').collect();

    // #################################################
    // save files
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
    let mut channel = sess.channel_session().unwrap();
    channel
        .exec(&cmd)
        // .exec("ls -l /sdcard/DCIM/Camera | awk '{print $8}'")
        .unwrap();
    println!("marker set!");
    println!("Total size: {}", &totalsize);
    println!("Total size: {}", totalsize / 1024 / 1024);

    // delete files older than 60 days
    // find . -type f -mtime +60 -exec rm -f {} \;
    let cmd = "cd ".to_owned()
        + &cfg.source.path
        + " && find . -type f -mtime +"
        + &cfg.source.clean.older_than_days
    // TODO: replace <file> with <rm -f> and protect this part with a cli flag to avoid accidental
    // deletion
        + " -exec file {} \\;";
    let mut channel = sess.channel_session().unwrap();
    channel
        .exec(&cmd)
        // .exec("ls -l /sdcard/DCIM/Camera | awk '{print $8}'")
        .unwrap();
    let mut s = String::new();
    channel.read_to_string(&mut s).unwrap();
    println!("{}", &s);
    bar.finish();
}
