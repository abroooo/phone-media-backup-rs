use chrono::NaiveDate;
use clap::{Parser, Subcommand};
use serde::de::Error;
use serde::{Deserialize, Deserializer, Serialize};
use std::env;
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
pub struct Args {
    #[arg(short, long, help = "Path to the configuration file.")]
    pub config: String,

    #[arg(
        short = 'n',
        long,
        global = true,
        help = "For the cleanup phase: list candidates instead of deleting. No effect on backup/verify."
    )]
    pub dry_run: bool,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Save new files from the phone (default when no subcommand is given)
    Backup(BackupArgs),
    /// Verify that files on the phone have been backed up (default: all)
    Verify(VerifyArgs),
    /// Delete source files older than `source.clean.older_than_days`
    Clean,
}

#[derive(clap::Args, Default)]
pub struct BackupArgs {
    #[arg(
        long,
        help = "After save, also run cleanup. Equivalent to chaining `backup` then `clean`."
    )]
    pub clean: bool,
}

#[derive(clap::Args, Default, Debug)]
pub struct VerifyArgs {
    #[arg(
        long,
        value_parser = parse_naive_date,
        help = "Only verify files with phone mtime on or after this date (YYYY-MM-DD)."
    )]
    pub from: Option<NaiveDate>,

    #[arg(
        long,
        value_parser = parse_naive_date,
        help = "Only verify files with phone mtime on or before this date (YYYY-MM-DD)."
    )]
    pub to: Option<NaiveDate>,
}

impl VerifyArgs {
    /// Reject `--from` later than `--to`.
    ///
    /// # Errors
    /// - if both bounds are set and `from > to`.
    pub fn validate(&self) -> Result<(), String> {
        if let (Some(f), Some(t)) = (self.from, self.to) {
            if f > t {
                return Err(format!("--from ({f}) must not be after --to ({t})"));
            }
        }
        Ok(())
    }
}

fn parse_naive_date(s: &str) -> Result<NaiveDate, String> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .map_err(|e| format!("invalid date {s:?} (expected YYYY-MM-DD): {e}"))
}

#[derive(Default, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MyConfig {
    pub general: General,
    pub source: Source,
    pub destination: Destination,
    #[serde(default)]
    pub notifier: Option<Notifier>,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Notifier {
    pub url: String,
    pub token: String,
}
#[derive(Default, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub user: String,
    pub ip: String,
    pub port: u16,
    #[serde(
        default = "default_known_hosts",
        deserialize_with = "deserialize_path_with_tilde"
    )]
    pub known_hosts: PathBuf,
    #[serde(
        default = "default_cert",
        deserialize_with = "deserialize_path_with_tilde"
    )]
    pub cert: PathBuf,
    #[serde(deserialize_with = "deserialize_remote_path")]
    pub path: PathBuf,
    pub clean: Clean,
}
#[derive(Default, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Destination {
    pub path: PathBuf,
}
#[derive(Default, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct General {
    pub mountpoint: PathBuf,
}
#[derive(Default, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Clean {
    #[serde(deserialize_with = "deserialize_older_than_days")]
    pub older_than_days: u16,
}

fn default_known_hosts() -> PathBuf {
    expand_tilde(Path::new("~/.ssh/known_hosts"))
}

fn default_cert() -> PathBuf {
    expand_tilde(Path::new("~/.ssh/id_ed25519"))
}

fn expand_tilde(path: &Path) -> PathBuf {
    let path_str = path.to_string_lossy();
    if let Some(rest) = path_str.strip_prefix("~") {
        if let Ok(home) = env::var("HOME") {
            let rest = rest.strip_prefix('/').unwrap_or(rest);
            return PathBuf::from(home).join(rest);
        }
        return PathBuf::from("/root/.ssh");
    }
    path.to_path_buf()
}

fn deserialize_path_with_tilde<'de, D>(deserializer: D) -> Result<PathBuf, D::Error>
where
    D: Deserializer<'de>,
{
    let path = PathBuf::deserialize(deserializer)?;
    Ok(expand_tilde(&path))
}

fn deserialize_remote_path<'de, D>(deserializer: D) -> Result<PathBuf, D::Error>
where
    D: Deserializer<'de>,
{
    let path = String::deserialize(deserializer)?;
    validate_remote_path(&path).map_err(D::Error::custom)?;
    Ok(PathBuf::from(path))
}

fn validate_remote_path(path: &str) -> Result<(), String> {
    if path.is_empty() {
        return Err("source.path is empty".into());
    }
    if !path.starts_with('/') {
        return Err(format!("source.path must be absolute, got {path:?}"));
    }
    if path.len() > 1 && path.ends_with('/') {
        return Err(format!("source.path must not end with '/', got {path:?}"));
    }
    for (i, c) in path.char_indices() {
        let ok = c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '_' | '-');
        if !ok {
            return Err(format!(
                "source.path contains disallowed character {c:?} at byte {i} \
                   (allowed: A-Z a-z 0-9 / . _ -)"
            ));
        }
    }
    if path.split('/').any(|seg| seg == "..") {
        return Err(format!(
            "source.path must not contain '..' segments, got {path:?}"
        ));
    }
    Ok(())
}

fn validate_older_than_days(days: u16) -> Result<(), String> {
    if days == 0 {
        return Err(format!(
            "source.clean.older_than_days must be positive, got {days}",
        ));
    }
    Ok(())
}

fn deserialize_older_than_days<'de, D>(deserializer: D) -> Result<u16, D::Error>
where
    D: Deserializer<'de>,
{
    let days = u16::deserialize(deserializer)?;
    validate_older_than_days(days).map_err(D::Error::custom)?;
    Ok(days)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_tilde_expands_against_home() {
        let home = env::var("HOME").expect("HOME must be set for this test");
        let home = PathBuf::from(home);

        assert_eq!(
            expand_tilde(Path::new("~/.ssh/known_hosts")),
            home.join(".ssh/known_hosts")
        );
        assert_eq!(expand_tilde(Path::new("~/foo")), home.join("foo"));
        assert_eq!(expand_tilde(Path::new("~")), home);
    }

    #[test]
    fn expand_tilde_no_tilde_passthrough() {
        assert_eq!(
            expand_tilde(Path::new("/etc/ssh/known_hosts")),
            PathBuf::from("/etc/ssh/known_hosts")
        );
        assert_eq!(
            expand_tilde(Path::new("relative/path")),
            PathBuf::from("relative/path")
        );
    }

    #[test]
    fn expand_tilde_only_at_start() {
        // A `~` mid-path is not an expansion target.
        assert_eq!(
            expand_tilde(Path::new("/etc/foo~bar")),
            PathBuf::from("/etc/foo~bar")
        );
    }

    #[test]
    fn validate_remote_path_accepts_typical() {
        assert!(validate_remote_path("/sdcard/DCIM/Camera").is_ok());
        assert!(validate_remote_path("/storage/emulated/0/DCIM").is_ok());
        assert!(validate_remote_path("/a").is_ok());
    }

    #[test]
    fn validate_remote_path_rejects_empty_and_relative() {
        assert!(validate_remote_path("").is_err());
        assert!(validate_remote_path("sdcard/DCIM").is_err());
        assert!(validate_remote_path("./x").is_err());
    }

    #[test]
    fn validate_remote_path_rejects_shell_metas() {
        for bad in [
            "/sdcard;rm -rf /",
            "/sdcard && cat /etc/passwd",
            "/sdcard`id`",
            "/sdcard$(id)",
            "/sdcard|cat",
            "/sdcard\nrm",
            "/sdcard'x",
            "/sdcard\"x",
            "/sdcard\\x",
            "/sdcard*",
            "/sdcard ",
            "/sdcard\t",
        ] {
            assert!(validate_remote_path(bad).is_err(), "should reject {bad:?}");
        }
    }

    #[test]
    fn validate_remote_path_rejects_dotdot_and_trailing_slash() {
        assert!(validate_remote_path("/sdcard/../etc").is_err());
        assert!(validate_remote_path("/sdcard/").is_err());
        assert!(validate_remote_path("/").is_ok()); // single-slash root allowed
    }

    #[test]
    fn validate_remote_path_rejects_control_chars() {
        assert!(validate_remote_path("/sdcard\x00").is_err());
        assert!(validate_remote_path("/sdcard\x1b[31m").is_err());
    }

    #[test]
    fn parse_naive_date_accepts_iso() {
        assert_eq!(
            parse_naive_date("2024-07-15").unwrap(),
            NaiveDate::from_ymd_opt(2024, 7, 15).unwrap()
        );
    }

    #[test]
    fn parse_naive_date_rejects_bad_input() {
        assert!(parse_naive_date("2024/07/15").is_err());
        assert!(parse_naive_date("15-07-2024").is_err());
        assert!(parse_naive_date("not-a-date").is_err());
        assert!(parse_naive_date("2024-13-01").is_err());
    }

    #[test]
    fn verify_args_validate_ok() {
        let a = VerifyArgs {
            from: NaiveDate::from_ymd_opt(2024, 1, 1),
            to: NaiveDate::from_ymd_opt(2024, 12, 31),
        };
        assert!(a.validate().is_ok());

        let a = VerifyArgs {
            from: NaiveDate::from_ymd_opt(2024, 6, 1),
            to: NaiveDate::from_ymd_opt(2024, 6, 1),
        };
        assert!(a.validate().is_ok(), "equal bounds allowed");

        let a = VerifyArgs {
            from: None,
            to: None,
        };
        assert!(a.validate().is_ok());

        let a = VerifyArgs {
            from: NaiveDate::from_ymd_opt(2024, 6, 1),
            to: None,
        };
        assert!(a.validate().is_ok(), "open upper bound allowed");

        let a = VerifyArgs {
            from: None,
            to: NaiveDate::from_ymd_opt(2024, 6, 1),
        };
        assert!(a.validate().is_ok(), "open lower bound allowed");
    }

    #[test]
    fn verify_args_validate_rejects_inverted_range() {
        let a = VerifyArgs {
            from: NaiveDate::from_ymd_opt(2024, 12, 31),
            to: NaiveDate::from_ymd_opt(2024, 1, 1),
        };
        let err = a.validate().unwrap_err();
        assert!(
            err.contains("--from"),
            "expected from/to message, got: {err}"
        );
    }
}
