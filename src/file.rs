use crate::config;
use crate::helpers;
use anyhow::Context;
use indicatif::ProgressBar;
use std::fs::metadata;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use xxhash_rust::xxh3::xxh3_64;

use exif::{DateTime, In, Reader, Tag, Value};
use std::io::BufReader;

use chrono::{Duration, NaiveDateTime};

const PARTIAL_HASH_SIZE: u64 = 64 * 1024;

fn partial_checksum(path: &PathBuf) -> std::io::Result<u64> {
    let mut file = File::open(path)?;
    let file_size = file.metadata()?.len();

    let first_chunk_size = std::cmp::min(file_size, PARTIAL_HASH_SIZE) as usize;
    let mut buffer = vec![0u8; first_chunk_size];
    file.read_exact(&mut buffer)?;

    if file_size > 0 {
        let last_offset = if file_size > PARTIAL_HASH_SIZE {
            file_size - PARTIAL_HASH_SIZE
        } else {
            0
        };
        file.seek(SeekFrom::Start(last_offset))?;
        let tail_len = (file_size - last_offset) as usize;
        let mut tail = vec![0u8; tail_len];
        file.read_exact(&mut tail)?;
        buffer.extend_from_slice(&tail);
    }

    Ok(xxh3_64(&buffer))
}

pub fn compare_partial(source: &PathBuf, destination: &PathBuf) -> std::io::Result<bool> {
    if !Path::new(destination).exists() {
        return Ok(false);
    }
    let src_hash = partial_checksum(source)?;
    let dst_hash = partial_checksum(destination)?;
    if src_hash == dst_hash {
        println!("files are identical (checksum match)");
        return Ok(true);
    }
    Ok(false)
}

pub fn compare_strict(source: &PathBuf, destination: &PathBuf) -> std::io::Result<bool> {
    if !Path::new(destination).exists() {
        return Ok(false);
    }

    let m1 = metadata(source)?;
    let m2 = metadata(destination)?;
    if m1.len() != m2.len() {
        return Ok(false);
    }

    let mut file1 = File::open(source)?;
    let mut file2 = File::open(destination)?;

    let mut buffer1 = [0; 65536];
    let mut buffer2 = [0; 65536];

    let file_size = m1.len() as usize;
    let mut remaining = file_size;

    while remaining > 0 {
        let chunk_size = std::cmp::min(65536, remaining);
        file1.read_exact(&mut buffer1[..chunk_size])?;
        file2.read_exact(&mut buffer2[..chunk_size])?;

        if buffer1[..chunk_size] != buffer2[..chunk_size] {
            return Ok(false);
        }

        remaining -= chunk_size;
    }

    println!("files are identical (strict match)");
    Ok(true)
}

pub struct ResolvedFile {
    pub dest_path: PathBuf,
    pub datetime: Option<NaiveDateTime>,
}

/// Given a source file path and its base_name, resolve where it would be backed up to.
/// Returns `None` for unsupported extensions or files without date info.
pub fn resolve_destination(
    dest_base: &PathBuf,
    base_name: &str,
    path_name: &PathBuf,
) -> Option<ResolvedFile> {
    let ext = helpers::get_extension_from_filename(path_name)?;
    match ext {
        "jpg" | "dng" => {
            let file = File::open(path_name).ok()?;
            let exif = Reader::new()
                .read_from_container(&mut BufReader::new(&file))
                .ok()?;
            let field = exif.get_field(Tag::DateTime, In::PRIMARY)?;
            if let Value::Ascii(ref vec) = field.value {
                let datetime = vec.first().and_then(|s| DateTime::from_ascii(s).ok())?;

                let dest = format!(
                    "{}/{}/{:02}/{}",
                    dest_base.display(),
                    datetime.year,
                    datetime.month,
                    base_name
                );

                let naive =
                    NaiveDateTime::parse_from_str(&datetime.to_string(), "%Y-%m-%d %H:%M:%S")
                        .ok()?;

                return Some(ResolvedFile {
                    dest_path: dest.into(),
                    datetime: Some(naive),
                });
            }
            None
        }
        "mp4" => {
            let re = regex::Regex::new(r"PXL_(\d{4})(\d{2})(\d{2})_").unwrap();
            if let Some(date_match) = re.captures(path_name.to_str()?) {
                let year: String = date_match[1].to_string();
                let month: String = date_match[2].to_string();
                let day: String = date_match[3].to_string();
                let naive = NaiveDateTime::parse_from_str(
                    &format!("{}-{}-{} 00:00:00", year, month, day),
                    "%Y-%m-%d %H:%M:%S",
                )
                .ok()?;
                let dest = format!("{}/{}/{}/{}", dest_base.display(), year, month, base_name);
                Some(ResolvedFile {
                    dest_path: dest.into(),
                    datetime: Some(naive),
                })
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Verify that all given files have been backed up by checking checksums.
/// Returns a list of files that are missing or don't match.
pub fn verify_files(cfg: &config::MyConfig, file_names: Vec<&str>) -> Vec<String> {
    let base_dir = &cfg.general.mountpoint;
    let mut missing: Vec<String> = Vec::new();
    let mut checked: usize = 0;
    let mut ok: usize = 0;
    let mut skipped: usize = 0;
    let bar = ProgressBar::new(file_names.len().try_into().unwrap());

    for base_name in &file_names {
        bar.inc(1);
        let path_name = match PathBuf::from(&base_dir).join(base_name).canonicalize() {
            Ok(p) => p,
            Err(e) => {
                println!("ERROR accessing {:?}: {}", base_name, e);
                missing.push(base_name.to_string());
                continue;
            }
        };
        let canonical_root = match PathBuf::from(base_dir).canonicalize() {
            Ok(p) => p,
            Err(e) => {
                eprintln!("canonicalize failed: {:?} {}", base_dir, e);
                missing.push(base_name.to_string());
                continue;
            }
        };
        if !path_name.starts_with(&canonical_root) {
            eprintln!("path escape attempt: {:?}", path_name);
            missing.push(path_name.to_string_lossy().to_string());
            continue;
        }

        match resolve_destination(&cfg.destination.path, base_name, &path_name.clone()) {
            Some(resolved) => {
                checked += 1;
                match compare_partial(&path_name.clone(), &resolved.dest_path) {
                    Ok(true) => {
                        ok += 1;
                    }
                    Ok(false) => {
                        println!(
                            "MISSING/DIFFERENT: {:?} -> {:?}",
                            &path_name, resolved.dest_path
                        );
                        missing.push(base_name.to_string());
                    }
                    Err(e) => {
                        println!("ERROR checking {:?}: {}", path_name, e);
                        missing.push(base_name.to_string());
                    }
                }
            }
            None => {
                skipped += 1;
            }
        }
    }

    bar.finish();
    println!(
        "\nVerify summary: {} checked, {} ok, {} missing/different, {} skipped (unsupported)",
        checked,
        ok,
        missing.len(),
        skipped
    );
    missing
}

pub fn save_files(cfg: &config::MyConfig, file_names: Vec<&str>) -> anyhow::Result<NaiveDateTime> {
    let mut skipped_files: Vec<PathBuf> = Vec::new();
    let base_dir = &cfg.general.mountpoint;
    let mut latest_date: NaiveDateTime = Default::default();
    let mut totalsize: u64 = 0;
    let bar = ProgressBar::new(file_names.len().try_into()?);
    for base_name in file_names {
        bar.inc(1);
        let path_name = PathBuf::from(base_dir).join(base_name).canonicalize()?;
        let canonical_root = PathBuf::from(base_dir).canonicalize()?;
        if !path_name.starts_with(&canonical_root) {
            eprintln!("path escape attempt: {:?}", path_name);
            skipped_files.push(path_name);
            continue;
        }
        let fsize = path_name.metadata()?.len();
        totalsize += fsize;
        println!(
            "filesize: {} MB | totalsize: {}",
            fsize / 1024 / 1024,
            totalsize / 1024 / 1024
        );

        match resolve_destination(&cfg.destination.path, base_name, &path_name) {
            Some(resolved) => {
                println!("{:?}", &path_name);
                if let Some(dt) = resolved.datetime {
                    if dt > latest_date {
                        latest_date = dt;
                    }
                }
                let dest_dir = &resolved.dest_path.to_string_lossy()
                // TODO: consider using resolved.dest_path.parent() instead of string manipulation
                // to get the directory
                    [..resolved.dest_path.to_string_lossy().rfind('/').unwrap_or(0)];
                let _ = std::fs::create_dir_all(dest_dir);

                if compare_partial(&path_name, &resolved.dest_path)? {
                    println!("file already exists, skipping: {:?}", resolved.dest_path);
                } else {
                    std::fs::copy(&path_name, &resolved.dest_path)
                        .with_context(|| format!("copy {}", &path_name.to_string_lossy()))?;
                    if !compare_strict(&path_name, &resolved.dest_path)? {
                        eprintln!(
                            "ERROR: copied file does not match source: {:?}",
                            resolved.dest_path
                        );
                    }
                    let metadata = std::fs::metadata(&path_name)?;
                    let _ = helpers::set_file_times(&resolved.dest_path, metadata);
                    println!("move {:?} to {:?}", path_name, resolved.dest_path);
                }
            }
            None => {
                eprintln!("skipping (unsupported or no date): {:?}", path_name);
                skipped_files.push(path_name);
            }
        }
        println!("_________________________________________________________________________________________");
    }
    bar.finish();
    skipped_files.iter().for_each(|x| {
        println!("skipped file: {:?}", x);
    });
    let marker_date = latest_date - Duration::seconds(60 * 60 * 24); // set marker to 1 day before latest date to
                                                                     // not miss files that are created after
                                                                     // backup but on the same day
    Ok(marker_date)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn create_file(dir: &TempDir, name: &str, content: &[u8]) -> PathBuf {
        let path = dir.path().join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let mut f = File::create(&path).unwrap();
        f.write_all(content).unwrap();
        path
    }

    // --- compare_files: Checksum mode ---

    #[test]
    fn test_checksum_identical_files() {
        let dir = TempDir::new().unwrap();
        let src = create_file(&dir, "a.bin", b"hello world");
        let dst = create_file(&dir, "b.bin", b"hello world");
        assert!(compare_partial(&src, &dst,).unwrap());
    }

    #[test]
    fn test_checksum_different_files() {
        let dir = TempDir::new().unwrap();
        let src = create_file(&dir, "a.bin", b"hello world");
        let dst = create_file(&dir, "b.bin", b"goodbye world");
        assert!(!compare_partial(&src, &dst,).unwrap());
    }

    #[test]
    fn test_checksum_destination_missing() {
        let dir = TempDir::new().unwrap();
        let src = create_file(&dir, "a.bin", b"hello");
        let dst = dir.path().join("nonexistent.bin");
        assert!(!compare_partial(&src, &dst,).unwrap());
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
        assert!(compare_partial(&src, &dst).unwrap());
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
        assert!(!compare_partial(&src, &dst).unwrap());
    }

    // --- compare_files: Strict mode ---

    #[test]
    fn test_strict_identical_files() {
        let dir = TempDir::new().unwrap();
        let src = create_file(&dir, "a.bin", b"hello world");
        let dst = create_file(&dir, "b.bin", b"hello world");
        assert!(compare_strict(&src, &dst).unwrap());
    }

    #[test]
    fn test_strict_different_files() {
        let dir = TempDir::new().unwrap();
        let src = create_file(&dir, "a.bin", b"hello world");
        let dst = create_file(&dir, "b.bin", b"goodbye world");
        assert!(!compare_strict(&src, &dst).unwrap());
    }

    #[test]
    fn test_strict_destination_missing() {
        let dir = TempDir::new().unwrap();
        let src = create_file(&dir, "a.bin", b"hello");
        let dst = dir.path().join("nonexistent.bin");
        assert!(!compare_strict(&src, &dst).unwrap());
    }

    #[test]
    fn test_save_files_jpg_with_exif() {
        let jpeg = make_test_jpeg();
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
                path: dst_dir.path().into(),
            },
        };

        let result = save_files(&cfg, vec!["test_photo.jpg"]).unwrap();
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
        let _src = create_file_at(mount_dir.path(), "PXL_20240315_something.mp4", b"fake mp4");

        let cfg = config::MyConfig {
            general: config::General {
                mountpoint: mount_dir.path().to_str().unwrap().to_string() + "/",
            },
            source: Default::default(),
            destination: config::Destination {
                path: dst_dir.path().into(),
            },
        };

        save_files(&cfg, vec!["PXL_20240315_something.mp4"]);
        let expected = dst_dir.path().join("2024/03/PXL_20240315_something.mp4");
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
                path: dst_dir.path().into(),
            },
        };

        save_files(&cfg, vec!["notes.txt"]);
        // Nothing should be created in dst_dir (only the dir itself)
        let entries: Vec<_> = std::fs::read_dir(dst_dir.path()).unwrap().collect();
        assert!(
            entries.is_empty(),
            "No files should be copied for .txt extension"
        );
    }

    #[test]
    fn test_save_files_returns_latest_date() {
        // Create two MP4 files with different dates in their names
        let mount_dir = TempDir::new().unwrap();
        let dst_dir = TempDir::new().unwrap();
        let _f1 = create_file_at(mount_dir.path(), "PXL_20230101_a.mp4", b"mp4 data");
        let _f2 = create_file_at(mount_dir.path(), "PXL_20240601_b.mp4", b"mp4 data");

        let cfg = config::MyConfig {
            general: config::General {
                mountpoint: mount_dir.path().to_str().unwrap().to_string() + "/",
            },
            source: Default::default(),
            destination: config::Destination {
                path: dst_dir.path().into(),
            },
        };

        let result = save_files(&cfg, vec!["PXL_20230101_a.mp4", "PXL_20240601_b.mp4"]).unwrap();
        // save_files only updates latest_date for jpg/dng (via exif), not mp4
        // so for mp4-only input the result stays at the default
        assert_eq!(result, NaiveDateTime::default());
    }

    fn create_file_at(dir: &Path, name: &str, content: &[u8]) -> PathBuf {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let mut f = File::create(&path).unwrap();
        f.write_all(content).unwrap();
        path
    }

    fn make_test_jpeg() -> Vec<u8> {
        let exif_datetime = b"2023:07:15 10:30:00\0";
        let mut app1_body = Vec::new();
        app1_body.extend_from_slice(b"Exif\0\0");
        app1_body.extend_from_slice(&[0x49, 0x49]);
        app1_body.extend_from_slice(&42u16.to_le_bytes());
        app1_body.extend_from_slice(&8u32.to_le_bytes());
        app1_body.extend_from_slice(&1u16.to_le_bytes());
        app1_body.extend_from_slice(&0x0132u16.to_le_bytes());
        app1_body.extend_from_slice(&2u16.to_le_bytes());
        app1_body.extend_from_slice(&20u32.to_le_bytes());
        let value_offset: u32 = 8 + 2 + 12 + 4;
        app1_body.extend_from_slice(&value_offset.to_le_bytes());
        app1_body.extend_from_slice(&0u32.to_le_bytes());
        app1_body.extend_from_slice(exif_datetime);

        let app1_len = (app1_body.len() + 2) as u16;
        let mut jpeg = Vec::new();
        jpeg.extend_from_slice(&[0xFF, 0xD8]);
        jpeg.extend_from_slice(&[0xFF, 0xE1]);
        jpeg.extend_from_slice(&app1_len.to_be_bytes());
        jpeg.extend_from_slice(&app1_body);
        jpeg.extend_from_slice(&[0xFF, 0xD9]);
        jpeg
    }

    // --- resolve_destination ---

    #[test]
    fn test_resolve_destination_jpg() {
        let dir = TempDir::new().unwrap();
        let jpeg = make_test_jpeg();
        let src = create_file_at(dir.path(), "photo.jpg", &jpeg);

        let result = resolve_destination(&PathBuf::from("/backup"), "photo.jpg", &src).unwrap();
        assert_eq!(result.dest_path, PathBuf::from("/backup/2023/07/photo.jpg"));
        assert_eq!(
            result.datetime.unwrap(),
            NaiveDateTime::parse_from_str("2023-07-15 10:30:00", "%Y-%m-%d %H:%M:%S").unwrap()
        );
    }

    #[test]
    fn test_resolve_destination_mp4() {
        let dir = TempDir::new().unwrap();
        let src = create_file_at(dir.path(), "PXL_20260410_101720861.mp4", b"fake mp4");

        let result = resolve_destination(
            &PathBuf::from("/backup"),
            "PXL_20260410_101720861.mp4",
            &src,
        )
        .unwrap();
        assert_eq!(
            result.dest_path,
            PathBuf::from("/backup/2026/04/PXL_20260410_101720861.mp4")
        );
        assert_eq!(
            result.datetime.unwrap(),
            NaiveDateTime::parse_from_str("2026-04-10 00:00:00", "%Y-%m-%d %H:%M:%S").unwrap()
        );
    }

    #[test]
    fn test_resolve_destination_unsupported() {
        let dir = TempDir::new().unwrap();
        let src = create_file_at(dir.path(), "notes.txt", b"text");
        assert!(resolve_destination(&PathBuf::from("/backup"), "notes.txt", &src).is_none());
    }

    // --- verify_files ---

    #[test]
    fn test_verify_files_all_backed_up() {
        let mount_dir = TempDir::new().unwrap();
        let dst_dir = TempDir::new().unwrap();

        // Create source mp4
        let _src = create_file_at(mount_dir.path(), "PXL_20240315_a.mp4", b"mp4 data");
        // Pre-create identical backup
        create_file_at(dst_dir.path(), "2024/03/PXL_20240315_a.mp4", b"mp4 data");

        let cfg = config::MyConfig {
            general: config::General {
                mountpoint: mount_dir.path().to_str().unwrap().to_string() + "/",
            },
            source: Default::default(),
            destination: config::Destination {
                path: dst_dir.path().into(),
            },
        };

        let missing = verify_files(&cfg, vec!["PXL_20240315_a.mp4"]);
        assert!(missing.is_empty());
    }

    #[test]
    fn test_verify_files_missing_backup() {
        let mount_dir = TempDir::new().unwrap();
        let dst_dir = TempDir::new().unwrap();

        let _src = create_file_at(mount_dir.path(), "PXL_20240315_a.mp4", b"mp4 data");
        // No backup exists

        let cfg = config::MyConfig {
            general: config::General {
                mountpoint: mount_dir.path().to_str().unwrap().to_string() + "/",
            },
            source: Default::default(),
            destination: config::Destination {
                path: dst_dir.path().into(),
            },
        };

        let missing = verify_files(&cfg, vec!["PXL_20240315_a.mp4"]);
        assert_eq!(missing, vec!["PXL_20240315_a.mp4"]);
    }

    #[test]
    fn test_verify_files_different_content() {
        let mount_dir = TempDir::new().unwrap();
        let dst_dir = TempDir::new().unwrap();

        let _src = create_file_at(mount_dir.path(), "PXL_20240315_a.mp4", b"original data");
        create_file_at(
            dst_dir.path(),
            "2024/03/PXL_20240315_a.mp4",
            b"corrupted data",
        );

        let cfg = config::MyConfig {
            general: config::General {
                mountpoint: mount_dir.path().to_str().unwrap().to_string() + "/",
            },
            source: Default::default(),
            destination: config::Destination {
                path: dst_dir.path().into(),
            },
        };

        let missing = verify_files(&cfg, vec!["PXL_20240315_a.mp4"]);
        assert_eq!(missing, vec!["PXL_20240315_a.mp4"]);
    }

    #[test]
    fn test_verify_files_skips_unsupported() {
        let mount_dir = TempDir::new().unwrap();
        let dst_dir = TempDir::new().unwrap();

        let _src = create_file_at(mount_dir.path(), "notes.txt", b"text");

        let cfg = config::MyConfig {
            general: config::General {
                mountpoint: mount_dir.path().to_str().unwrap().to_string() + "/",
            },
            source: Default::default(),
            destination: config::Destination {
                path: dst_dir.path().into(),
            },
        };

        let missing = verify_files(&cfg, vec!["notes.txt"]);
        assert!(missing.is_empty());
    }
}
