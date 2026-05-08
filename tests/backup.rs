use phone_media_backup_rs::{config, file, run_backup, run_clean, ssh::Phone};
use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

#[derive(Default)]
struct FakePhone {
    marker: RefCell<String>,
    files: Vec<String>,
    cleanup_calls: RefCell<Vec<(u16, bool)>>,
}

impl Phone for FakePhone {
    fn marker(&self) -> anyhow::Result<String> {
        Ok(self.marker.borrow().clone())
    }
    fn list_new(&self, _since: &str) -> anyhow::Result<Vec<String>> {
        Ok(self.files.clone())
    }
    fn list_all(&self) -> anyhow::Result<Vec<String>> {
        Ok(self.files.clone())
    }
    fn set_marker(&self, m: &str) -> anyhow::Result<()> {
        *self.marker.borrow_mut() = m.to_string();
        Ok(())
    }
    fn cleanup(&self, days: u16, dry_run: bool) -> anyhow::Result<()> {
        self.cleanup_calls.borrow_mut().push((days, dry_run));
        Ok(())
    }
}

fn make_cfg(mount: &Path, dest: &Path) -> config::MyConfig {
    config::MyConfig {
        general: config::General {
            mountpoint: mount.to_path_buf(),
        },
        source: config::Source {
            user: String::new(),
            ip: String::new(),
            port: 0,
            known_hosts: PathBuf::new(),
            cert: PathBuf::new(),
            path: "/test".into(),
            clean: config::Clean {
                older_than_days: 30,
            },
        },
        destination: config::Destination {
            path: dest.to_path_buf(),
        },
        notifier: None,
    }
}

#[test]
fn backup_copies_and_advances_marker() {
    let mount = TempDir::new().unwrap();
    let dest = TempDir::new().unwrap();

    let src = mount.path().join("PXL_20240115_120000000.mp4");
    fs::write(&src, b"fake video bytes").unwrap();

    let cfg = make_cfg(mount.path(), dest.path());
    let phone = FakePhone {
        marker: RefCell::new("1970-01-01".into()),
        files: vec!["./PXL_20240115_120000000.mp4".into()],
        ..Default::default()
    };

    run_backup(&cfg, &phone).unwrap();

    let copied = dest.path().join("2024/01/PXL_20240115_120000000.mp4");
    assert!(copied.exists(), "expected file at {}", copied.display());
    assert_eq!(fs::read(&copied).unwrap(), b"fake video bytes");
    assert_eq!(*phone.marker.borrow(), "2024-01-14");

    // idempotent: re-run, file already there → compare_partial short-circuits copy
    let mtime_before = fs::metadata(&copied).unwrap().modified().unwrap();
    run_backup(&cfg, &phone).unwrap();
    let mtime_after = fs::metadata(&copied).unwrap().modified().unwrap();
    assert_eq!(mtime_before, mtime_after, "second run should not rewrite");
}

#[test]
fn backup_rejects_path_outside_mount() {
    let mount = TempDir::new().unwrap();
    let dest = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    fs::write(outside.path().join("PXL_20240115_120000000.mp4"), b"x").unwrap();

    // symlink inside mount → file outside mount
    std::os::unix::fs::symlink(
        outside.path().join("PXL_20240115_120000000.mp4"),
        mount.path().join("escape.mp4"),
    )
    .unwrap();

    let cfg = make_cfg(mount.path(), dest.path());
    let phone = FakePhone {
        marker: RefCell::new("1970-01-01".into()),
        files: vec!["./escape.mp4".into()],
        ..Default::default()
    };

    // save_one bails per-file; run_backup silently swallows it (the path is logged
    // as "skipped" but not surfaced in the return value — existing behavior).
    // Post-empty-list-fix: with no dated file processed, latest_date is None and
    // set_marker is never called, so the marker stays at its prior value.
    run_backup(&cfg, &phone).unwrap();
    assert!(fs::read_dir(dest.path()).unwrap().next().is_none());
    assert_eq!(*phone.marker.borrow(), "1970-01-01");
}

#[test]
fn marker_advances_to_latest_minus_one() {
    let mount = TempDir::new().unwrap();
    let dest = TempDir::new().unwrap();

    for name in [
        "PXL_20231201_120000000.mp4",
        "PXL_20240315_120000000.mp4",
        "PXL_20240120_120000000.mp4",
    ] {
        fs::write(mount.path().join(name), b"video").unwrap();
    }

    let cfg = make_cfg(mount.path(), dest.path());
    let phone = FakePhone {
        marker: RefCell::new("1970-01-01".into()),
        files: vec![
            "./PXL_20231201_120000000.mp4".into(),
            "./PXL_20240315_120000000.mp4".into(),
            "./PXL_20240120_120000000.mp4".into(),
        ],
        ..Default::default()
    };

    run_backup(&cfg, &phone).unwrap();

    assert_eq!(*phone.marker.borrow(), "2024-03-14");
    assert!(dest
        .path()
        .join("2023/12/PXL_20231201_120000000.mp4")
        .exists());
    assert!(dest
        .path()
        .join("2024/03/PXL_20240315_120000000.mp4")
        .exists());
    assert!(dest
        .path()
        .join("2024/01/PXL_20240120_120000000.mp4")
        .exists());
}

#[test]
fn unsupported_files_skipped() {
    let mount = TempDir::new().unwrap();
    let dest = TempDir::new().unwrap();

    fs::write(mount.path().join("notes.txt"), b"hello").unwrap();
    fs::write(mount.path().join("PXL_20240115_120000000.mp4"), b"video").unwrap();

    let cfg = make_cfg(mount.path(), dest.path());
    let phone = FakePhone {
        marker: RefCell::new("1970-01-01".into()),
        files: vec!["./notes.txt".into(), "./PXL_20240115_120000000.mp4".into()],
        ..Default::default()
    };

    run_backup(&cfg, &phone).unwrap();

    assert_eq!(*phone.marker.borrow(), "2024-01-14");
    assert!(dest
        .path()
        .join("2024/01/PXL_20240115_120000000.mp4")
        .exists());
    // .txt should not appear anywhere under dest
    let txt_present =
        walkdir(dest.path()).any(|p| p.extension().and_then(|s| s.to_str()) == Some("txt"));
    assert!(!txt_present, "unsupported .txt file leaked into dest tree");
}

#[test]
fn invalid_marker_format_errors() {
    let mount = TempDir::new().unwrap();
    let dest = TempDir::new().unwrap();
    let cfg = make_cfg(mount.path(), dest.path());
    let phone = FakePhone {
        marker: RefCell::new("garbage".into()),
        files: vec![],
        ..Default::default()
    };

    let err = run_backup(&cfg, &phone).unwrap_err();
    assert!(
        err.to_string().contains("invalid date format"),
        "got: {err}"
    );
}

#[test]
fn empty_marker_treated_as_epoch() {
    let mount = TempDir::new().unwrap();
    let dest = TempDir::new().unwrap();
    fs::write(mount.path().join("PXL_20240115_120000000.mp4"), b"video").unwrap();

    let cfg = make_cfg(mount.path(), dest.path());
    let phone = FakePhone {
        marker: RefCell::new(String::new()),
        files: vec!["./PXL_20240115_120000000.mp4".into()],
        ..Default::default()
    };

    run_backup(&cfg, &phone).unwrap();
    assert_eq!(*phone.marker.borrow(), "2024-01-14");
}

// `cat marker` on the phone returns the date with a trailing newline.
// is_valid_date_format's regex tolerates `\r?\n?$` and run_backup trims before
// using the date.
#[test]
fn trailing_newline_in_marker_tolerated() {
    let mount = TempDir::new().unwrap();
    let dest = TempDir::new().unwrap();
    fs::write(mount.path().join("PXL_20240115_120000000.mp4"), b"video").unwrap();

    let cfg = make_cfg(mount.path(), dest.path());
    let phone = FakePhone {
        marker: RefCell::new("1970-01-01\n".into()),
        files: vec!["./PXL_20240115_120000000.mp4".into()],
        ..Default::default()
    };

    run_backup(&cfg, &phone).unwrap();
    assert_eq!(*phone.marker.borrow(), "2024-01-14");
}

// Whitespace-only marker pins current behavior: `is_empty()` is false, regex
// rejects, run_backup bails. If we ever want to treat whitespace-only the same
// as truly empty (cat-of-empty-file → epoch), flip this assertion and change
// `marker.is_empty()` in lib.rs to `marker.trim().is_empty()`.
#[test]
fn whitespace_only_marker_bails() {
    let mount = TempDir::new().unwrap();
    let dest = TempDir::new().unwrap();
    let cfg = make_cfg(mount.path(), dest.path());
    let phone = FakePhone {
        marker: RefCell::new("\n".into()),
        files: vec![],
        ..Default::default()
    };

    let err = run_backup(&cfg, &phone).unwrap_err();
    assert!(
        err.to_string().contains("invalid date format"),
        "got: {err}"
    );
}

#[test]
fn verify_finds_no_missing_after_backup() {
    let mount = TempDir::new().unwrap();
    let dest = TempDir::new().unwrap();
    fs::write(mount.path().join("PXL_20240115_120000000.mp4"), b"video").unwrap();

    let cfg = make_cfg(mount.path(), dest.path());
    let phone = FakePhone {
        marker: RefCell::new("1970-01-01".into()),
        files: vec!["./PXL_20240115_120000000.mp4".into()],
        ..Default::default()
    };

    run_backup(&cfg, &phone).unwrap();

    let files = phone.list_all().unwrap();
    let refs: Vec<&str> = files.iter().map(String::as_str).collect();
    let report = file::verify_files(&cfg, &refs);
    assert!(report.is_clean(), "unexpected failures: {report:?}");
}

#[test]
fn verify_reports_all_missing_when_dest_empty() {
    let mount = TempDir::new().unwrap();
    let dest = TempDir::new().unwrap();
    fs::write(mount.path().join("PXL_20240115_120000000.mp4"), b"video").unwrap();

    let cfg = make_cfg(mount.path(), dest.path());
    let phone = FakePhone {
        files: vec!["./PXL_20240115_120000000.mp4".into()],
        ..Default::default()
    };

    let files = phone.list_all().unwrap();
    let refs: Vec<&str> = files.iter().map(String::as_str).collect();
    let report = file::verify_files(&cfg, &refs);
    assert_eq!(report.missing.len(), 1);
    assert!(report.mismatched.is_empty());
}

#[test]
fn cleanup_forwards_days_and_dry_run() {
    let mount = TempDir::new().unwrap();
    let dest = TempDir::new().unwrap();
    let mut cfg = make_cfg(mount.path(), dest.path());
    cfg.source.clean.older_than_days = 42;

    let phone = FakePhone::default();

    run_clean(&cfg, &phone, true).unwrap();
    run_clean(&cfg, &phone, false).unwrap();

    assert_eq!(*phone.cleanup_calls.borrow(), vec![(42, true), (42, false)]);
}

#[test]
fn empty_file_list_does_not_clobber_marker() {
    let mount = TempDir::new().unwrap();
    let dest = TempDir::new().unwrap();
    let cfg = make_cfg(mount.path(), dest.path());
    let phone = FakePhone {
        marker: RefCell::new("2024-06-01".into()),
        files: vec![],
        ..Default::default()
    };

    run_backup(&cfg, &phone).unwrap();
    assert_eq!(*phone.marker.borrow(), "2024-06-01");
}

fn walkdir(root: &std::path::Path) -> Box<dyn Iterator<Item = PathBuf>> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(p) = stack.pop() {
        let Ok(rd) = fs::read_dir(&p) else { continue };
        for e in rd.flatten() {
            let path = e.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push(path);
            }
        }
    }
    Box::new(out.into_iter())
}
