use ssh2::Session;
use std::io::{Cursor, Read, Seek, SeekFrom};
use std::net::TcpStream;
use std::process::Command;
extern crate exif;

use std::env;
use std::ffi::OsStr;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::time::SystemTime;

use exif::{DateTime, In, Reader, Tag, Value};

use md5::{Digest, Md5};

fn compare_files(file1_path: &str, file2_path: &str) -> std::io::Result<bool> {
    let mut file1 = File::open(file1_path)?;
    let mut file2 = File::open(file2_path)?;

    let mut buffer1 = [0; 4096];
    let mut buffer2 = [0; 4096];

    loop {
        let bytes_read1 = file1.read(&mut buffer1)?;
        let bytes_read2 = file2.read(&mut buffer2)?;

        if bytes_read1 != bytes_read2 || buffer1[..bytes_read1] != buffer2[..bytes_read2] {
            return Ok(false);
        }

        if bytes_read1 == 0 {
            break;
        }
    }
    println!("files are identical");

    Ok(true)
}

fn calculate_md5(filename: &str) -> Vec<u8> {
    let mut file = File::open(filename).expect("Unable to open file");
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer).expect("Unable to read file");
    let mut hasher = Md5::new();
    hasher.update(&buffer);
    hasher.finalize().to_vec()
}
fn get_extension_from_filename(filename: &str) -> Option<&str> {
    Path::new(filename).extension().and_then(OsStr::to_str)
}
fn set_file_times(dest_file: &str, metadata: std::fs::Metadata) -> std::io::Result<()> {
    filetime::set_file_times(
        &dest_file,
        filetime::FileTime::from_last_access_time(&metadata),
        filetime::FileTime::from_last_modification_time(&metadata),
    )?;
    Ok(())
}

fn move_file(dloc: DateLoc) {
    // let destination_base = "/home/user/dev/rust/media_backup/test_destination";
    // let destination_base = "/mnt/backup/photos";
    let destination_base = "/home/user/tmp";
    let dest_dir = [destination_base, &dloc.year, &dloc.month].join("/");
    let dest_file = [
        destination_base,
        &dloc.year,
        &dloc.month.to_string(),
        &dloc.base_name,
    ]
    .join("/");
    let _ = std::fs::create_dir_all(&dest_dir);

    // println!("{}", filename);
    let _ = std::fs::copy(&dloc.source, &dest_file);
    let _ = compare_files(&dloc.source, &dest_file);
    // let source_md5 = calculate_md5(&dloc.source);
    // let dest_md5 = calculate_md5(&dest_file);
    //   if source_md5 == dest_md5 {
    //     println!("Files are identical.");
    // } else {
    //     println!("Files are different.");
    // }
    let metadata = std::fs::metadata(&dloc.source).unwrap();
    let _ = set_file_times(&dest_file, metadata);
    println!("move {} to {}", dloc.source, dest_file);
}

struct DateLoc {
    month: String,
    year: String,
    source: String,
    base_name: String,
}

fn main() {
    // Command::new("mkdir -p /home/user/dev/rust/phone-media-backup-rs/sshmount && sshfs -p 2222 192.168.1.10:/sdcard/DCIM/Camera /home/user/dev/rust/phone-media-backup-rs/sshmount")
    //     .spawn();

    // create ssh session
    let remote_host = "192.168.1.10";
    let remote_user = "user";
    let remote_file_path = "/sdcard/DCIM/Camera/IMG_20231022_124817.jpg";
    let local_file_path = "file.jpg";
    let private_key_path = "/home/user/.ssh/id_ed25519";
    // let remote_host = "1.1.0.2";
    // let remote_user = "your_username";
    // let private_key_path = "/path/to/your/private_key";
    let private_key_path = Path::new(private_key_path);

    // Connect to the remote SSH server
    let tcp = TcpStream::connect(format!("{}:2222", remote_host)).unwrap();
    let mut sess = Session::new().unwrap();
    sess.set_tcp_stream(tcp);
    sess.handshake().unwrap();

    // Authenticate using a certificate (public key)
    sess.userauth_pubkey_file(remote_user, None, private_key_path, None)
        .unwrap();

    // Check if authentication was successful
    assert!(sess.authenticated());

    // #################################################
    // get start date and build command
    let mut channel = sess.channel_session().unwrap();
    channel
        .exec("cd /sdcard/DCIM/Camera && cat marker")
        // .exec("ls -l /sdcard/DCIM/Camera | awk '{print $8}'")
        .unwrap();
    let mut s = String::new();
    channel.read_to_string(&mut s).unwrap();
    println!("{}", &s);
    let command = "cd /sdcard/DCIM/Camera && find . -type f -newermt ".to_owned() + &s;
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
    let baseDir = "/home/user/sshmount/";
    for f in file_names {
        let path_name = (baseDir.to_owned() + f.as_ref());
        // println!("name: {}", path_name);
        println!(
            "filesize: {}",
            std::path::PathBuf::from(path_name)
                .metadata()
                .unwrap()
                .len()
        );
    }
    // for entry in std::fs::read_dir("/home/user/sshmount").unwrap() {
    //     let tmp = &entry.as_ref().unwrap().path();
    //     let base_name = &entry.unwrap().path().file_name().unwrap().to_owned();
    //     let t = tmp.to_string_lossy().to_owned();
    //     let x = get_extension_from_filename(&t);
    //     match x {
    //         Some(x) => {
    //             match x {
    //                 "jpg" => {
    //                     println!("{:?}", &t);
    //                     let file = File::open(&t.to_string()).unwrap();
    //                     let exif = Reader::new().read_from_container(
    //                         &mut BufReader::new(&file)).unwrap();
    //                     if let Some(field) = exif.get_field(Tag::DateTime, In::PRIMARY) {
    //                         match field.value {
    //                             Value::Ascii(ref vec) if !vec.is_empty() => {
    //                                 if let Ok(datetime) = DateTime::from_ascii(&vec[0]) {
    //                                     println!("Year of DateTime is {}.", &datetime.year);
    //                                     println!("Month of DateTime is {:02}.", &datetime.month);
    //                                     // println!("Day of DateTime is {}.", datetime.day);
    //                                     move_file(DateLoc{year: datetime.year.to_string(),
    //                                                         month: format!("{:02}", datetime.month),
    //                                                         source: t.to_string(),
    //                                                         base_name: base_name.to_str().unwrap().to_string()});
    //                                 }
    //                             },
    //                             _ => {},
    //                         }
    //                     }
    //                 }
    //                 "mp4" => {
    //                     println!("{:?}", &t);
    //                     let re = regex::Regex::new(r"\d{8}").unwrap();
    //                     if let Some(date_match) = re.find(&t) {
    //                         let year: String = date_match.as_str().chars().take(4).collect();
    //                         let month: String = date_match.as_str().chars().skip(4).take(2).collect();
    //                         println!("year: {}",&year.to_string());
    //                         println!("month: {}",&month.to_string());
    //                                     move_file(DateLoc{year: year.to_string(),
    //                                                         month: month.to_string(),
    //                                                         source: t.to_string(),
    //                                                         base_name: base_name.to_str().unwrap().to_string()});
    //                     } else {
    //                         println!("No date found in the string.");
    //                     }
    //                 }
    //                 _ => {
    //                     println!("dont care");
    //                 }
    //             }
    //         }
    //         None => {
    //             println!("dont care {}",t);
    //         }
    //     }
    //
    // }
}
