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
    /// Verify that all files on the phone have been backed up
    Verify,
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
    pub path: String,
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

fn deserialize_remote_path<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let path = String::deserialize(deserializer)?;
    validate_remote_path(&path).map_err(D::Error::custom)?;
    Ok(path)
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
}
