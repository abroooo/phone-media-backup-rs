#![warn(clippy::all, clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]

pub mod config;
pub mod file;
pub mod helpers;
pub mod notifier;
pub mod ssh;

pub use ssh::{Mount, Phone, RealPhone, Ssh};

use chrono::{Days, NaiveDate};
use tracing::{debug, info, warn};

/// Run the backup process: list new files on the phone, save them to the destination, and update
/// the marker.
/// # Errors
///
/// - invalid date format (not YYYY-MM-DD)
/// - communication errors with phone (e.g., SSH errors)
/// - I/O errors when saving files to the destination
pub fn run_backup(cfg: &config::MyConfig, phone: &dyn Phone) -> anyhow::Result<()> {
    let notifier = cfg
        .notifier
        .as_ref()
        .map(|n| notifier::BackupNotifier::new(&n.url, &n.token));

    if let Some(n) = &notifier {
        if let Err(e) = n.notify("in progress") {
            warn!("Failed to send 'in progress' notification: {e}");
        }
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

    let file_names = phone.list_range(Some(&date), None)?;
    let refs: Vec<&str> = file_names.iter().map(String::as_str).collect();
    if let Some(latest_date) = file::save_files(cfg, refs)? {
        let new_marker = latest_date.format("%Y-%m-%d").to_string();
        info!("Set marker {}", &new_marker);
        phone.set_marker(&new_marker)?;
    } else {
        info!("No dated files processed; leaving marker unchanged");
    }

    if let Some(n) = &notifier {
        if let Err(e) = n.notify("success") {
            warn!("Failed to send 'success' notification: {e}");
        }
    }
    Ok(())
}

/// Run the verification process: list files on the phone (optionally restricted
/// to a date range based on phone-side mtime) and check that they have been
/// backed up. Both bounds are inclusive at day granularity; the upper bound is
/// converted to a half-open `to+1` for the underlying `find -newermt` filter.
///
/// # Errors
///
/// - communication errors with phone (e.g., SSH errors)
pub fn run_verify(
    cfg: &config::MyConfig,
    phone: &dyn Phone,
    from: Option<NaiveDate>,
    to: Option<NaiveDate>,
) -> anyhow::Result<()> {
    let from_str = from.map(|d| d.format("%Y-%m-%d").to_string());
    // Inclusive --to D becomes exclusive `find ! -newermt (D+1)`. If D+1
    // overflows NaiveDate's range, drop the upper bound — effectively open.
    let to_excl_str = to
        .and_then(|d| d.checked_add_days(Days::new(1)))
        .map(|d| d.format("%Y-%m-%d").to_string());

    match (&from_str, &to) {
        (Some(f), Some(t)) => info!("Verifying files in [{f} .. {t}] (inclusive, phone mtime)"),
        (Some(f), None) => info!("Verifying files from {f} onwards (inclusive, phone mtime)"),
        (None, Some(t)) => info!("Verifying files up to {t} (inclusive, phone mtime)"),
        (None, None) => {}
    }

    let file_names = phone.list_range(from_str.as_deref(), to_excl_str.as_deref())?;
    let refs: Vec<&str> = file_names.iter().map(String::as_str).collect();
    info!("Verifying {} files", refs.len());
    let report = file::verify_files(cfg, &refs);
    if report.is_clean() {
        info!("All files verified");
    } else {
        if !report.missing.is_empty() {
            info!("{} files MISSING (no backup):", report.missing.len());
            for f in &report.missing {
                info!("  {f}");
            }
        }
        if !report.mismatched.is_empty() {
            info!(
                "{} files MISMATCHED (backup content differs):",
                report.mismatched.len()
            );
            for f in &report.mismatched {
                info!("  {f}");
            }
        }
        if !report.errors.is_empty() {
            info!("{} files could not be checked:", report.errors.len());
            for f in &report.errors {
                info!("  {f}");
            }
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
