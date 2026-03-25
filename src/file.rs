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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn create_file(dir: &TempDir, name: &str, content: &[u8]) -> String {
        let path = dir.path().join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let mut f = File::create(&path).unwrap();
        f.write_all(content).unwrap();
        path.to_str().unwrap().to_string()
    }

    // --- compare_files: Checksum mode ---

    #[test]
    fn test_checksum_identical_files() {
        let dir = TempDir::new().unwrap();
        let src = create_file(&dir, "a.bin", b"hello world");
        let dst = create_file(&dir, "b.bin", b"hello world");
        assert!(compare_files(&src, &dst, CompareMode::Checksum).unwrap());
    }

    #[test]
    fn test_checksum_different_files() {
        let dir = TempDir::new().unwrap();
        let src = create_file(&dir, "a.bin", b"hello world");
        let dst = create_file(&dir, "b.bin", b"goodbye world");
        assert!(!compare_files(&src, &dst, CompareMode::Checksum).unwrap());
    }

    #[test]
    fn test_checksum_destination_missing() {
        let dir = TempDir::new().unwrap();
        let src = create_file(&dir, "a.bin", b"hello");
        let dst = dir.path().join("nonexistent.bin");
        assert!(!compare_files(&src, dst.to_str().unwrap(), CompareMode::Checksum).unwrap());
    }

    #[test]
    fn test_checksum_large_file() {
        let dir = TempDir::new().unwrap();
        // 200 kB file — both first and last 64 kB chunks are read
        let mut data = vec![0xABu8; 200 * 1024];
        // Make it non-uniform so the tail chunk matters
        for (i, byte) in data.iter_mut().enumerate() {
            *byte = (i % 251) as u8;
        }
        let src = create_file(&dir, "big_a.bin", &data);
        let dst = create_file(&dir, "big_b.bin", &data);
        assert!(compare_files(&src, &dst, CompareMode::Checksum).unwrap());
    }

    #[test]
    fn test_checksum_large_file_differ_at_end() {
        let dir = TempDir::new().unwrap();
        let mut data = vec![0u8; 200 * 1024];
        for (i, byte) in data.iter_mut().enumerate() {
            *byte = (i % 251) as u8;
        }
        let src = create_file(&dir, "big_a.bin", &data);

        // Flip last byte
        let mut data2 = data.clone();
        let len = data2.len();
        data2[len - 1] ^= 0xFF;
        let dst = create_file(&dir, "big_b.bin", &data2);
        assert!(!compare_files(&src, &dst, CompareMode::Checksum).unwrap());
    }

    // --- compare_files: Strict mode ---

    #[test]
    fn test_strict_identical_files() {
        let dir = TempDir::new().unwrap();
        let src = create_file(&dir, "a.bin", b"hello world");
        let dst = create_file(&dir, "b.bin", b"hello world");
        assert!(compare_files(&src, &dst, CompareMode::Strict).unwrap());
    }

    #[test]
    fn test_strict_different_files() {
        let dir = TempDir::new().unwrap();
        let src = create_file(&dir, "a.bin", b"hello world");
        let dst = create_file(&dir, "b.bin", b"goodbye world");
        assert!(!compare_files(&src, &dst, CompareMode::Strict).unwrap());
    }

    #[test]
    fn test_strict_destination_missing() {
        let dir = TempDir::new().unwrap();
        let src = create_file(&dir, "a.bin", b"hello");
        let dst = dir.path().join("nonexistent.bin");
        assert!(!compare_files(&src, dst.to_str().unwrap(), CompareMode::Strict).unwrap());
    }

    // --- move_file ---

    #[test]
    fn test_move_file_copies_to_correct_path() {
        let src_dir = TempDir::new().unwrap();
        let dst_dir = TempDir::new().unwrap();
        let src = create_file(&src_dir, "photo.jpg", b"fake image data");

        let dloc = helpers::DateLoc {
            year: "2024".to_string(),
            month: "03".to_string(),
            source: src.clone(),
            base_name: "photo.jpg".to_string(),
            destination_base: dst_dir.path().to_str().unwrap().to_string(),
        };
        move_file(dloc);

        let expected = dst_dir.path().join("2024/03/photo.jpg");
        assert!(expected.exists(), "File should exist at {:?}", expected);
        assert_eq!(std::fs::read(&expected).unwrap(), b"fake image data");
    }

    #[test]
    fn test_move_file_skips_identical() {
        let src_dir = TempDir::new().unwrap();
        let dst_dir = TempDir::new().unwrap();
        let content = b"identical content";
        let src = create_file(&src_dir, "photo.jpg", content);

        // Pre-create identical file at destination
        let dest_path = dst_dir.path().join("2024/03/photo.jpg");
        std::fs::create_dir_all(dest_path.parent().unwrap()).unwrap();
        std::fs::write(&dest_path, content).unwrap();

        let dloc = helpers::DateLoc {
            year: "2024".to_string(),
            month: "03".to_string(),
            source: src.clone(),
            base_name: "photo.jpg".to_string(),
            destination_base: dst_dir.path().to_str().unwrap().to_string(),
        };
        move_file(dloc);

        // File should still be identical (no error, no corruption)
        assert_eq!(std::fs::read(&dest_path).unwrap(), content);
    }

    #[test]
    fn test_move_file_preserves_timestamps() {
        let src_dir = TempDir::new().unwrap();
        let dst_dir = TempDir::new().unwrap();
        let src = create_file(&src_dir, "photo.jpg", b"fake image data");

        // Set a known modification time on the source
        let known_mtime = filetime::FileTime::from_unix_time(1_700_000_000, 0);
        filetime::set_file_mtime(&src, known_mtime).unwrap();

        let dloc = helpers::DateLoc {
            year: "2024".to_string(),
            month: "03".to_string(),
            source: src.clone(),
            base_name: "photo.jpg".to_string(),
            destination_base: dst_dir.path().to_str().unwrap().to_string(),
        };
        move_file(dloc);

        let dest_path = dst_dir.path().join("2024/03/photo.jpg");
        let dest_mtime = filetime::FileTime::from_last_modification_time(&std::fs::metadata(&dest_path).unwrap());
        assert_eq!(dest_mtime, known_mtime);
    }

    // --- save_files ---

    #[test]
    fn test_save_files_jpg_with_exif() {
        // Minimal JPEG with EXIF DateTime tag for 2023-07-15 10:30:00
        // Built from: SOI, APP1 (Exif header + IFD with DateTime tag), EOI
        let exif_datetime = b"2023:07:15 10:30:00\0";
        let mut app1_body = Vec::new();
        // Exif header
        app1_body.extend_from_slice(b"Exif\0\0");
        // TIFF header: little-endian
        app1_body.extend_from_slice(&[0x49, 0x49]); // "II"
        app1_body.extend_from_slice(&42u16.to_le_bytes()); // magic
        app1_body.extend_from_slice(&8u32.to_le_bytes()); // offset to IFD0
        // IFD0: 1 entry
        app1_body.extend_from_slice(&1u16.to_le_bytes()); // count
        // Tag: DateTime (0x0132), ASCII (2), count=20, offset
        app1_body.extend_from_slice(&0x0132u16.to_le_bytes());
        app1_body.extend_from_slice(&2u16.to_le_bytes()); // ASCII
        app1_body.extend_from_slice(&20u32.to_le_bytes()); // count (including null)
        // Value offset: 8 (TIFF header) + 2 (count) + 12 (entry) + 4 (next IFD) = 26
        let value_offset: u32 = 8 + 2 + 12 + 4;
        app1_body.extend_from_slice(&value_offset.to_le_bytes());
        // Next IFD pointer
        app1_body.extend_from_slice(&0u32.to_le_bytes());
        // DateTime value
        app1_body.extend_from_slice(exif_datetime);

        let app1_len = (app1_body.len() + 2) as u16; // +2 for length field itself
        let mut jpeg = Vec::new();
        jpeg.extend_from_slice(&[0xFF, 0xD8]); // SOI
        jpeg.extend_from_slice(&[0xFF, 0xE1]); // APP1 marker
        jpeg.extend_from_slice(&app1_len.to_be_bytes());
        jpeg.extend_from_slice(&app1_body);
        jpeg.extend_from_slice(&[0xFF, 0xD9]); // EOI

        let mount_dir = TempDir::new().unwrap();
        let dst_dir = TempDir::new().unwrap();

        // Write the JPEG into mount_dir
        let _src = create_file_at(mount_dir.path(), "test_photo.jpg", &jpeg);

        let cfg = config::MyConfig {
            general: config::General {
                mountpoint: mount_dir.path().to_str().unwrap().to_string() + "/",
            },
            source: Default::default(),
            destination: config::Destination {
                path: dst_dir.path().to_str().unwrap().to_string(),
            },
        };

        let result = save_files(&cfg, vec!["test_photo.jpg"]);
        let expected = dst_dir.path().join("2023/07/test_photo.jpg");
        assert!(expected.exists(), "JPEG should be saved at {:?}", expected);
        assert_eq!(
            result,
            NaiveDateTime::parse_from_str("2023-07-15 10:30:00", "%Y-%m-%d %H:%M:%S").unwrap()
        );
    }

    #[test]
    fn test_save_files_mp4_date_from_filename() {
        let mount_dir = TempDir::new().unwrap();
        let dst_dir = TempDir::new().unwrap();
        let _src = create_file_at(mount_dir.path(), "VID_20240315_something.mp4", b"fake mp4");

        let cfg = config::MyConfig {
            general: config::General {
                mountpoint: mount_dir.path().to_str().unwrap().to_string() + "/",
            },
            source: Default::default(),
            destination: config::Destination {
                path: dst_dir.path().to_str().unwrap().to_string(),
            },
        };

        save_files(&cfg, vec!["VID_20240315_something.mp4"]);
        let expected = dst_dir.path().join("2024/03/VID_20240315_something.mp4");
        assert!(expected.exists(), "MP4 should be saved at {:?}", expected);
    }

    #[test]
    fn test_save_files_skips_unknown_extension() {
        let mount_dir = TempDir::new().unwrap();
        let dst_dir = TempDir::new().unwrap();
        let _src = create_file_at(mount_dir.path(), "notes.txt", b"some text");

        let cfg = config::MyConfig {
            general: config::General {
                mountpoint: mount_dir.path().to_str().unwrap().to_string() + "/",
            },
            source: Default::default(),
            destination: config::Destination {
                path: dst_dir.path().to_str().unwrap().to_string(),
            },
        };

        save_files(&cfg, vec!["notes.txt"]);
        // Nothing should be created in dst_dir (only the dir itself)
        let entries: Vec<_> = std::fs::read_dir(dst_dir.path()).unwrap().collect();
        assert!(entries.is_empty(), "No files should be copied for .txt extension");
    }

    #[test]
    fn test_save_files_returns_latest_date() {
        // Create two MP4 files with different dates in their names
        let mount_dir = TempDir::new().unwrap();
        let dst_dir = TempDir::new().unwrap();
        let _f1 = create_file_at(mount_dir.path(), "VID_20230101_a.mp4", b"mp4 data");
        let _f2 = create_file_at(mount_dir.path(), "VID_20240601_b.mp4", b"mp4 data");

        let cfg = config::MyConfig {
            general: config::General {
                mountpoint: mount_dir.path().to_str().unwrap().to_string() + "/",
            },
            source: Default::default(),
            destination: config::Destination {
                path: dst_dir.path().to_str().unwrap().to_string(),
            },
        };

        let result = save_files(&cfg, vec!["VID_20230101_a.mp4", "VID_20240601_b.mp4"]);
        // save_files only updates latest_date for jpg/dng (via exif), not mp4
        // so for mp4-only input the result stays at the default
        assert_eq!(result, NaiveDateTime::default());
    }

    fn create_file_at(dir: &Path, name: &str, content: &[u8]) -> String {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let mut f = File::create(&path).unwrap();
        f.write_all(content).unwrap();
        path.to_str().unwrap().to_string()
    }
}
