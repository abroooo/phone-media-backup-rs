use crate::config;
use crate::helpers;
use anyhow::Context;
use indicatif::ProgressStyle;
use std::fs::metadata;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use tracing::{error, info, info_span};
use tracing_indicatif::span_ext::IndicatifSpanExt;
use xxhash_rust::xxh3::xxh3_64;

use exif::{DateTime, In, Reader, Tag, Value};
use std::io::BufReader;

use chrono::{Duration, NaiveDateTime};

const PARTIAL_HASH_SIZE: u64 = 64 * 1024;
const STRICT_BUFFER_SIZE: usize = 65536;
static PXL_RE: std::sync::LazyLock<regex::Regex> =
    std::sync::LazyLock::new(|| regex::Regex::new(r"PXL_(\d{4})(\d{2})(\d{2})_").unwrap());

fn partial_checksum(path: &Path) -> std::io::Result<u64> {
    let mut file = File::open(path)?;
    let file_size = file.metadata()?.len();

    let first_chunk_size =
        usize::try_from(std::cmp::min(file_size, PARTIAL_HASH_SIZE)).unwrap_or(0);
    let mut buffer = vec![0u8; first_chunk_size];
    file.read_exact(&mut buffer)?;

    if file_size > 0 {
        let last_offset = file_size.saturating_sub(PARTIAL_HASH_SIZE);
        file.seek(SeekFrom::Start(last_offset))?;
        let tail_len = usize::try_from(file_size - last_offset).unwrap_or(0);
        let mut tail = vec![0u8; tail_len];
        file.read_exact(&mut tail)?;
        buffer.extend_from_slice(&tail);
    }

    Ok(xxh3_64(&buffer))
}

pub fn compare_partial(source: &Path, destination: &Path) -> std::io::Result<bool> {
    if !destination.exists() {
        return Ok(false);
    }
    let src_hash = partial_checksum(source)?;
    let dst_hash = partial_checksum(destination)?;
    if src_hash == dst_hash {
        info!("files are identical (checksum match)");
        return Ok(true);
    }
    Ok(false)
}

pub fn compare_strict(source: &Path, destination: &Path) -> std::io::Result<bool> {
    if !destination.exists() {
        return Ok(false);
    }

    let m1 = metadata(source)?;
    let m2 = metadata(destination)?;
    if m1.len() != m2.len() {
        return Ok(false);
    }

    let mut file1 = File::open(source)?;
    let mut file2 = File::open(destination)?;

    let mut buffer1 = vec![0; STRICT_BUFFER_SIZE].into_boxed_slice();
    let mut buffer2 = vec![0; STRICT_BUFFER_SIZE].into_boxed_slice();

    let file_size = usize::try_from(m1.len()).unwrap_or(0);
    let mut remaining = file_size;

    while remaining > 0 {
        let chunk_size = std::cmp::min(STRICT_BUFFER_SIZE, remaining);
        file1.read_exact(&mut buffer1[..chunk_size])?;
        file2.read_exact(&mut buffer2[..chunk_size])?;

        if buffer1[..chunk_size] != buffer2[..chunk_size] {
            return Ok(false);
        }

        remaining -= chunk_size;
    }

    info!("files are identical (strict match)");
    Ok(true)
}

pub struct ResolvedFile {
    pub dest_path: PathBuf,
    pub datetime: Option<NaiveDateTime>,
}

/// Given a source file path and its `base_name`, resolve where it would be backed up to.
/// Returns `None` for unsupported extensions or files without date info.
pub fn resolve_destination(
    dest_base: &Path,
    base_name: &str,
    path_name: &Path,
) -> Option<ResolvedFile> {
    let ext = helpers::get_extension_from_filename(path_name)?;
    match ext {
        // TODO: add support for more formats (e.g. mov, heic). Consider using a trait like
        // trait MediaKind {
        // fn matches(ext: &str) -> bool;
        // fn resolve(path: &Path, base: &str, dest: &str) -> Option<ResolvedFile>;
        // }
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
            if let Some(date_match) = PXL_RE.captures(path_name.to_str()?) {
                let year: String = date_match[1].to_string();
                let month: String = date_match[2].to_string();
                let day: String = date_match[3].to_string();
                let naive = NaiveDateTime::parse_from_str(
                    &format!("{year}-{month}-{day} 00:00:00"),
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
pub fn verify_files(cfg: &config::MyConfig, file_names: &[&str]) -> Vec<String> {
    let base_dir = &cfg.general.mountpoint;
    let mut missing: Vec<String> = Vec::new();
    let mut checked: usize = 0;
    let mut ok: usize = 0;
    let mut skipped: usize = 0;
    let span = info_span!("verify");
    span.pb_set_style(&ProgressStyle::with_template("{wide_bar} {pos}/{len} {msg}").unwrap());
    span.pb_set_length(file_names.len() as u64);
    let _enter = span.enter();

    for base_name in file_names {
        span.pb_inc(1);
        let path_name = match PathBuf::from(&base_dir).join(base_name).canonicalize() {
            Ok(p) => p,
            Err(e) => {
                error!("ERROR accessing {base_name}: {e}");
                missing.push(base_name.to_string());
                continue;
            }
        };
        let canonical_root = match PathBuf::from(base_dir).canonicalize() {
            Ok(p) => p,
            Err(e) => {
                error!("canonicalize failed: {} {e}", base_dir.display());
                missing.push(base_name.to_string());
                continue;
            }
        };
        if !path_name.starts_with(&canonical_root) {
            error!("path escape attempt: {}", path_name.display());
            missing.push(path_name.to_string_lossy().to_string());
            continue;
        }

        match resolve_destination(&cfg.destination.path, base_name, &path_name) {
            Some(resolved) => {
                checked += 1;
                match compare_partial(&path_name, &resolved.dest_path) {
                    Ok(true) => {
                        ok += 1;
                    }
                    Ok(false) => {
                        info!(
                            "MISSING/DIFFERENT: {} -> {}",
                            &path_name.display(),
                            resolved.dest_path.display()
                        );
                        missing.push(base_name.to_string());
                    }
                    Err(e) => {
                        error!("ERROR checking {}: {}", path_name.display(), e);
                        missing.push(base_name.to_string());
                    }
                }
            }
            None => {
                skipped += 1;
            }
        }
    }

    info!(
        "\nVerify summary: {} checked, {} ok, {} missing/different, {} skipped (unsupported)",
        checked,
        ok,
        missing.len(),
        skipped
    );
    missing
}

enum FileOutcome {
    Skipped,
    Processed(Option<NaiveDateTime>),
}

fn save_one(
    cfg: &config::MyConfig,
    base_name: &str,
    canonical_root: &Path,
    totalsize: &mut u64,
) -> anyhow::Result<FileOutcome> {
    let base_dir = &cfg.general.mountpoint;
    let path_name = PathBuf::from(base_dir)
        .join(base_name)
        .canonicalize()
        .with_context(|| format!("canonicalize {base_name}"))?;
    if !path_name.starts_with(canonical_root) {
        anyhow::bail!("path escape: {}", path_name.display());
    }
    let fsize = path_name
        .metadata()
        .with_context(|| format!("metadata {}", path_name.display()))?
        .len();
    *totalsize += fsize;
    info!(
        "filesize: {} MB | totalsize: {}",
        fsize / 1024 / 1024,
        *totalsize / 1024 / 1024
    );

    let Some(resolved) = resolve_destination(&cfg.destination.path, base_name, &path_name) else {
        info!("skipping (unsupported or no date): {}", path_name.display());
        return Ok(FileOutcome::Skipped);
    };

    info!("{}", path_name.display());
    let dest_dir = resolved
        .dest_path
        .parent()
        .with_context(|| format!("no parent for {}", resolved.dest_path.display()))?;
    std::fs::create_dir_all(dest_dir)
        .with_context(|| format!("create_dir_all {}", dest_dir.display()))?;

    if compare_partial(&path_name, &resolved.dest_path)
        .with_context(|| format!("compare_partial {}", path_name.display()))?
    {
        info!(
            "file already exists, skipping: {}",
            resolved.dest_path.display()
        );
    } else {
        info!(
            "copying {} to {}",
            path_name.display(),
            resolved.dest_path.display()
        );
        std::fs::copy(&path_name, &resolved.dest_path)
            .with_context(|| format!("copy {}", path_name.display()))?;
        if !compare_strict(&path_name, &resolved.dest_path)
            .with_context(|| format!("compare_strict {}", path_name.display()))?
        {
            if let Err(e) = std::fs::remove_file(&resolved.dest_path) {
                error!(
                    "Failed to remove corrupted file {}: {}",
                    resolved.dest_path.display(),
                    e
                );
            }
            anyhow::bail!(
                "copied file does not match source: {}",
                resolved.dest_path.display()
            );
        }
        let metadata = std::fs::metadata(&path_name)
            .with_context(|| format!("metadata {}", path_name.display()))?;
        helpers::set_file_times(&resolved.dest_path, &metadata)?;
        info!(
            "move {} to {}",
            path_name.display(),
            resolved.dest_path.display()
        );
    }
    Ok(FileOutcome::Processed(resolved.datetime))
}

pub fn save_files(cfg: &config::MyConfig, file_names: Vec<&str>) -> anyhow::Result<NaiveDateTime> {
    let mut skipped_files: Vec<String> = Vec::new();
    let base_dir = &cfg.general.mountpoint;
    let canonical_root = PathBuf::from(base_dir)
        .canonicalize()
        .with_context(|| format!("canonicalize root {}", base_dir.display()))?;
    let mut latest_date: NaiveDateTime = NaiveDateTime::default();
    let mut totalsize: u64 = 0;
    let span = info_span!("save_files");
    span.pb_set_style(&ProgressStyle::with_template("{wide_bar} {pos}/{len} {msg}").unwrap());
    span.pb_set_length(file_names.len() as u64);
    let _enter = span.enter();
    for base_name in file_names {
        span.pb_inc(1);
        match save_one(cfg, base_name, &canonical_root, &mut totalsize) {
            Ok(FileOutcome::Processed(Some(dt))) => {
                if dt > latest_date {
                    latest_date = dt;
                }
            }
            Ok(FileOutcome::Processed(None)) => {}
            Ok(FileOutcome::Skipped) => {
                skipped_files.push(base_name.to_string());
            }
            Err(e) => {
                error!("skip {base_name}: {e:#}");
                skipped_files.push(base_name.to_string());
            }
        }
        info!("_________________________________________________________________________________________");
    }
    span.pb_set_finish_message("Done!");
    for x in &skipped_files {
        info!("skipped file: {x}");
    }
    Ok(latest_date - Duration::days(1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Source;
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
            *byte = u8::try_from(i % 251).unwrap_or(0);
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
            *byte = u8::try_from(i % 251).unwrap_or(0);
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
                mountpoint: mount_dir.path().to_path_buf(),
            },
            source: Source::default(),
            destination: config::Destination {
                path: dst_dir.path().into(),
            },
        };

        let result = save_files(&cfg, vec!["test_photo.jpg"]).unwrap();
        let expected = dst_dir.path().join("2023/07/test_photo.jpg");
        assert!(expected.exists(), "JPEG should be saved at {expected:?}");
        // `make_test_jpeg` sets the EXIF DateTime to "2023:07:15 10:30:00", so we expect the
        // result to be that minus 1 day (as per save_files logic)
        assert_eq!(
            result,
            NaiveDateTime::parse_from_str("2023-07-14 10:30:00", "%Y-%m-%d %H:%M:%S").unwrap()
        );
    }

    #[test]
    fn test_save_files_mp4_date_from_filename() {
        let mount_dir = TempDir::new().unwrap();
        let dst_dir = TempDir::new().unwrap();
        let _src = create_file_at(mount_dir.path(), "PXL_20240315_something.mp4", b"fake mp4");

        let cfg = config::MyConfig {
            general: config::General {
                mountpoint: mount_dir.path().to_path_buf(),
            },
            source: Source::default(),
            destination: config::Destination {
                path: dst_dir.path().into(),
            },
        };

        let _ = save_files(&cfg, vec!["PXL_20240315_something.mp4"]);
        let expected = dst_dir.path().join("2024/03/PXL_20240315_something.mp4");
        assert!(expected.exists(), "MP4 should be saved at {expected:?}");
    }

    #[test]
    fn test_save_files_skips_unknown_extension() {
        let mount_dir = TempDir::new().unwrap();
        let dst_dir = TempDir::new().unwrap();
        let _src = create_file_at(mount_dir.path(), "notes.txt", b"some text");

        let cfg = config::MyConfig {
            general: config::General {
                mountpoint: mount_dir.path().to_path_buf(),
            },
            source: Source::default(),
            destination: config::Destination {
                path: dst_dir.path().into(),
            },
        };

        let _ = save_files(&cfg, vec!["notes.txt"]);
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
                mountpoint: mount_dir.path().to_path_buf(),
            },
            source: Source::default(),
            destination: config::Destination {
                path: dst_dir.path().into(),
            },
        };

        let result = save_files(&cfg, vec!["PXL_20230101_a.mp4", "PXL_20240601_b.mp4"]).unwrap();
        // save_files only updates latest_date for jpg/dng (via exif), not mp4
        // so for mp4-only input the result stays at the default
        assert_eq!(
            result,
            NaiveDateTime::parse_from_str("2024-05-31 00:00:00", "%Y-%m-%d %H:%M:%S").unwrap()
        );
    }

    #[test]
    fn test_save_files_continues_on_missing_file() {
        let jpeg = make_test_jpeg();
        let mount_dir = TempDir::new().unwrap();
        let dst_dir = TempDir::new().unwrap();
        let _src = create_file_at(mount_dir.path(), "good.jpg", &jpeg);

        let cfg = config::MyConfig {
            general: config::General {
                mountpoint: mount_dir.path().to_path_buf(),
            },
            source: Source::default(),
            destination: config::Destination {
                path: dst_dir.path().into(),
            },
        };

        let result = save_files(&cfg, vec!["does_not_exist.jpg", "good.jpg"]).unwrap();
        let expected = dst_dir.path().join("2023/07/good.jpg");
        assert!(
            expected.exists(),
            "good file should be saved despite earlier bad path"
        );
        assert_eq!(
            result,
            NaiveDateTime::parse_from_str("2023-07-14 10:30:00", "%Y-%m-%d %H:%M:%S").unwrap()
        );
    }

    #[test]
    fn test_save_files_mixed_returns_latest_from_good() {
        // Bad path between two good files: run completes, marker reflects the latest good date.
        let jpeg = make_test_jpeg();
        let mount_dir = TempDir::new().unwrap();
        let dst_dir = TempDir::new().unwrap();
        let _early = create_file_at(mount_dir.path(), "PXL_20220101_x.mp4", b"mp4 data");
        let _late = create_file_at(mount_dir.path(), "later.jpg", &jpeg);

        let cfg = config::MyConfig {
            general: config::General {
                mountpoint: mount_dir.path().to_path_buf(),
            },
            source: Source::default(),
            destination: config::Destination {
                path: dst_dir.path().into(),
            },
        };

        let result =
            save_files(&cfg, vec!["PXL_20220101_x.mp4", "missing.jpg", "later.jpg"]).unwrap();

        assert!(dst_dir.path().join("2022/01/PXL_20220101_x.mp4").exists());
        assert!(dst_dir.path().join("2023/07/later.jpg").exists());
        // jpg's 2023-07-15 10:30:00 wins over mp4's 2022-01-01 00:00:00, minus 1 day.
        assert_eq!(
            result,
            NaiveDateTime::parse_from_str("2023-07-14 10:30:00", "%Y-%m-%d %H:%M:%S").unwrap()
        );
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

        let app1_len = u16::try_from(app1_body.len() + 2).unwrap_or(0);
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
                mountpoint: mount_dir.path().to_path_buf(),
            },
            source: Source::default(),
            destination: config::Destination {
                path: dst_dir.path().into(),
            },
        };

        let missing = verify_files(&cfg, &["PXL_20240315_a.mp4"]);
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
                mountpoint: mount_dir.path().to_path_buf(),
            },
            source: Source::default(),
            destination: config::Destination {
                path: dst_dir.path().into(),
            },
        };

        let missing = verify_files(&cfg, &["PXL_20240315_a.mp4"]);
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
                mountpoint: mount_dir.path().to_path_buf(),
            },
            source: Source::default(),
            destination: config::Destination {
                path: dst_dir.path().into(),
            },
        };

        let missing = verify_files(&cfg, &["PXL_20240315_a.mp4"]);
        assert_eq!(missing, vec!["PXL_20240315_a.mp4"]);
    }

    #[test]
    fn test_verify_files_skips_unsupported() {
        let mount_dir = TempDir::new().unwrap();
        let dst_dir = TempDir::new().unwrap();

        let _src = create_file_at(mount_dir.path(), "notes.txt", b"text");

        let cfg = config::MyConfig {
            general: config::General {
                mountpoint: mount_dir.path().to_path_buf(),
            },
            source: Source::default(),
            destination: config::Destination {
                path: dst_dir.path().into(),
            },
        };

        let missing = verify_files(&cfg, &["notes.txt"]);
        assert!(missing.is_empty());
    }
}
