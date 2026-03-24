use crate::config;
use crate::helpers;
use indicatif::ProgressBar;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use xxhash_rust::xxh3::xxh3_64;

use exif::{DateTime, In, Reader, Tag, Value};
use std::io::BufReader;

use chrono::NaiveDateTime;

const PARTIAL_HASH_SIZE: u64 = 64 * 1024;

pub enum CompareMode {
    Checksum,
    Strict,
}

fn partial_checksum(path: &str) -> std::io::Result<u64> {
    let mut file = File::open(path)?;
    let file_size = file.metadata()?.len();

    let first_chunk_size = std::cmp::min(file_size, PARTIAL_HASH_SIZE) as usize;
    let mut buffer = vec![0u8; first_chunk_size];
    file.read_exact(&mut buffer)?;

    if file_size > PARTIAL_HASH_SIZE {
        let last_offset = file_size - PARTIAL_HASH_SIZE;
        if last_offset > PARTIAL_HASH_SIZE {
            file.seek(SeekFrom::Start(last_offset))?;
            let mut tail = vec![0u8; PARTIAL_HASH_SIZE as usize];
            file.read_exact(&mut tail)?;
            buffer.extend_from_slice(&tail);
        }
    }

    Ok(xxh3_64(&buffer))
}

pub fn compare_files(source: &str, destination: &str, mode: CompareMode) -> std::io::Result<bool> {
    if !Path::new(destination).exists() {
        return Ok(false);
    }

    match mode {
        CompareMode::Checksum => {
            let src_hash = partial_checksum(source)?;
            let dst_hash = partial_checksum(destination)?;
            if src_hash == dst_hash {
                println!("files are identical (checksum match)");
                return Ok(true);
            }
            Ok(false)
        }
        CompareMode::Strict => {
            let mut file1 = File::open(source)?;
            let mut file2 = File::open(destination)?;

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
            println!("files are identical (strict match)");
            Ok(true)
        }
    }
}
pub fn move_file(dloc: crate::helpers::DateLoc) {
    let destination_base = &dloc.destination_base.as_str();
    let dest_dir = [
        destination_base.to_string(),
        dloc.year.to_string(),
        dloc.month.to_string(),
    ]
    .join("/");
    let dest_file = [
        destination_base.to_string(),
        dloc.year.to_string(),
        dloc.month.to_string(),
        dloc.base_name,
    ]
    .join("/");
    let _ = std::fs::create_dir_all(&dest_dir);

    // println!("{}", filename);
    if compare_files(&dloc.source, &dest_file, CompareMode::Checksum).unwrap() {
        println!("file already exists, skipping: {}", dest_file);
    } else {
        let _ = std::fs::copy(&dloc.source, &dest_file);
        let _ = compare_files(&dloc.source, &dest_file, CompareMode::Strict);
        // let source_md5 = calculate_md5(&dloc.source);
        // let dest_md5 = calculate_md5(&dest_file);
        //   if source_md5 == dest_md5 {
        //     println!("Files are identical.");
        // } else {
        //     println!("Files are different.");
        // }
        let metadata = std::fs::metadata(&dloc.source).unwrap();
        let _ = helpers::set_file_times(&dest_file, metadata);
        println!("move {} to {}", dloc.source, dest_file);
    }
}

pub fn save_files(cfg: &config::MyConfig, file_names: Vec<&str>) -> NaiveDateTime {
    let mut skipped_files: Vec<String> = Vec::new();
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
                    let exif = match Reader::new().read_from_container(&mut BufReader::new(&file)) {
                        Ok(exif_data) => exif_data,
                        Err(e) => {
                            eprintln!("Failed to read EXIF data for {}: {} - skip", &t, e);
                            skipped_files.push(t.to_string());
                            continue;
                        }
                    };
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
                                    move_file(helpers::DateLoc {
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
                        move_file(helpers::DateLoc {
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
    skipped_files.iter().for_each(|x| {
        println!("skipped file: {}", x);
    });
    latest_date
}
