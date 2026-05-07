#![warn(clippy::all, clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]

pub mod config;
pub mod file;
pub mod helpers;
pub mod notifier;
pub mod ssh;

pub use ssh::{Mount, Phone, RealPhone, Ssh};

use tracing::{debug, info};

/// Run the backup process: list new files on the phone, save them to the destination, and update
/// the marker.
/// # Errors
///
/// - invalid date format (not YYYY-MM-DD)
/// - communication errors with phone (e.g., SSH errors)
/// - I/O errors when saving files to the destination
/// - notification errors (e.g., HTTP errors)
pub fn run_backup(cfg: &config::MyConfig, phone: &dyn Phone) -> anyhow::Result<()> {
    let notifier = cfg
        .notifier
        .as_ref()
        .map(|n| notifier::BackupNotifier::new(&n.url, &n.token));

    if let Some(n) = &notifier {
        n.notify("in progress")?;
    }

    let marker = phone.marker()?;
    let date = if marker.is_empty() {
        info!("Marker empty, defaulting to 1970-01-01");
        "1970-01-01".to_string()
    } else if !is_valid_date_format(&marker) {
        anyhow::bail!("Marker has invalid date format. Expected YYYY-MM-DD");
    } else {
        debug!("Marker: {}", &marker);
        marker.trim().to_string()
    };

    let file_names = phone.list_new(&date)?;
    let refs: Vec<&str> = file_names.iter().map(String::as_str).collect();
    if let Some(latest_date) = file::save_files(cfg, refs)? {
        let new_marker = latest_date.format("%Y-%m-%d").to_string();
        info!("Set marker {}", &new_marker);
        phone.set_marker(&new_marker)?;
    } else {
        info!("No dated files processed; leaving marker unchanged");
    }

    if let Some(n) = &notifier {
        n.notify("success")?;
    }
    Ok(())
}

/// Run the verification process: list all files on the phone and check that they have been backed
/// up.
///
/// # Errors
///
/// - communication errors with phone (e.g., SSH errors)
pub fn run_verify(cfg: &config::MyConfig, phone: &dyn Phone) -> anyhow::Result<()> {
    let file_names = phone.list_all()?;
    let refs: Vec<&str> = file_names.iter().map(String::as_str).collect();
    info!("Verifying {} files", refs.len());
    let missing = file::verify_files(cfg, &refs);
    if missing.is_empty() {
        info!("All files verified");
    } else {
        info!("{} files NOT backed up:", missing.len());
        for f in &missing {
            info!("  {f}");
        }
    }
    Ok(())
}

/// Run the cleanup process: list files on the phone older than `cfg.source.clean.older_than_days`
/// and delete them (or list them if `dry_run` is true).
///
/// # Errors
///
/// - communication errors with phone (e.g., SSH errors)
pub fn run_clean(cfg: &config::MyConfig, phone: &dyn Phone, dry_run: bool) -> anyhow::Result<()> {
    phone.cleanup(cfg.source.clean.older_than_days, dry_run)
}

fn is_valid_date_format(date_str: &str) -> bool {
    let re = regex::Regex::new(r"^\d{4}-\d{2}-\d{2}\r?\n?$").unwrap();
    re.is_match(date_str)
}
