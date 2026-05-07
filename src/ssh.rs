use crate::config;
use anyhow::Context;
use ssh2::Session;
use std::io::Read;
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::Command;
use tracing::{debug, error, info};

/// POSIX shell single-quote escape. Wraps `s` in `'...'` and replaces any
/// embedded `'` with the standard `'\''` sequence, so the result is safe to
/// interpolate into a `sh -c` command line — `sshfs` quirks aside, `libssh2`
/// runs `exec` through the remote login shell, which is POSIX on Android.
fn shell_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        if c == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(c);
        }
    }
    out.push('\'');
    out
}

/// Implementations that dispatch to a remote shell must escape any string
/// argument they interpolate into a command. Callers may pass arbitrary
/// strings; impls bear the escaping burden.
pub trait Phone {
    /// Get the current marker from the phone, which should be a date string in YYYY-MM-DD format.
    /// May return an empty string when the marker is missing or empty — depends on remote cat behavior.
    /// The marker is used to determine which files are new since the last backup.
    ///
    /// # Errors
    ///
    /// - communication errors with phone (e.g., SSH errors)
    fn marker(&self) -> anyhow::Result<String>;

    /// List files on the phone that are newer than the given date (in YYYY-MM-DD format). The date
    /// is typically obtained from the marker. Returns a vector of file paths relative to the
    /// phone's base path.
    ///
    /// # Errors
    ///
    /// - communication errors with phone (e.g., SSH errors)
    fn list_new(&self, since: &str) -> anyhow::Result<Vec<String>>;

    /// List all files on the phone, regardless of modification date. Returns a vector of file
    /// paths relative to the phone's base path.
    ///
    /// # Errors
    ///
    /// - communication errors with phone (e.g., SSH errors)
    fn list_all(&self) -> anyhow::Result<Vec<String>>;

    /// Set the marker on the phone to the given date string (in YYYY-MM-DD format). This is
    /// typically called after a successful backup to update the marker to the latest backup date.
    ///
    /// # Errors
    ///
    /// - communication errors with phone (e.g., SSH errors)
    fn set_marker(&self, marker: &str) -> anyhow::Result<()>;

    /// Clean up files on the phone that are older than the specified number of days. If `dry_run`
    /// is true, the method should only list the files that would be deleted without actually
    /// deleting them.
    ///
    /// # Errors
    ///
    /// - communication errors with phone (e.g., SSH errors)
    /// - I/O errors
    fn cleanup(&self, older_than_days: u16, dry_run: bool) -> anyhow::Result<()>;
}

pub struct RealPhone<'a> {
    ssh: &'a Ssh,
    path: &'a str,
}
impl<'a> RealPhone<'a> {
    #[must_use]
    pub fn new(ssh: &'a Ssh, path: &'a str) -> Self {
        Self { ssh, path }
    }
}

impl Phone for RealPhone<'_> {
    fn marker(&self) -> anyhow::Result<String> {
        self.ssh
            .execute_command(&format!("cd {} && cat marker", shell_escape(self.path)))
    }

    fn list_new(&self, since: &str) -> anyhow::Result<Vec<String>> {
        let out = self.ssh.execute_command(&format!(
            "cd {} && find . -type f -newermt {}",
            shell_escape(self.path),
            shell_escape(since)
        ))?;
        Ok(out
            .lines()
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect())
    }

    fn list_all(&self) -> anyhow::Result<Vec<String>> {
        let out = self
            .ssh
            .execute_command(&format!("cd {} && find . -type f", shell_escape(self.path)))?;
        Ok(out
            .lines()
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect())
    }

    fn set_marker(&self, marker: &str) -> anyhow::Result<()> {
        let cmd = format!(
            "cd {} && printf '%s\n' {} > marker",
            shell_escape(self.path),
            shell_escape(marker)
        );
        self.ssh.execute_command(&cmd)?;
        Ok(())
    }

    fn cleanup(&self, older_than_days: u16, dry_run: bool) -> anyhow::Result<()> {
        let action = if dry_run {
            "-print"
        } else {
            "-exec rm -f {{}} \\;"
        };
        let mode = if dry_run { "dry-run list" } else { "delete" };
        info!(
            "cleanup ({mode}): files older than {} days under {}",
            &older_than_days, self.path
        );
        let cmd = format!(
            "cd {} && find . -type f -mtime +{} {}",
            shell_escape(self.path),
            older_than_days,
            action
        );
        self.ssh.execute_command(&cmd)?;
        Ok(())
    }
}

pub struct Mount {
    mountpoint: PathBuf,
}
impl Mount {
    /// Mount the remote filesystem via sshfs at the configured mountpoint.     
    ///
    /// # Errors
    ///
    /// - issues executing sshfs (e.g., command not found,
    ///   permission issues, mount failures)
    pub fn new(cfg: &config::MyConfig) -> anyhow::Result<Self> {
        let out = Command::new("sshfs")
            .arg("-p")
            .arg(cfg.source.port.to_string())
            .arg(format!("{}:{}", cfg.source.ip, &cfg.source.path))
            .arg(&cfg.general.mountpoint)
            .output()?;
        if !out.status.success() {
            anyhow::bail!("Mount failed: {}", String::from_utf8_lossy(&out.stderr));
        }
        Ok(Self {
            mountpoint: PathBuf::from(&cfg.general.mountpoint),
        })
    }
}
impl Drop for Mount {
    /// Unmounts on drop. Errors are logged but never propagated, since `Drop`
    /// can't return them.
    fn drop(&mut self) {
        let r = Command::new("umount").arg(&self.mountpoint).output();
        match r {
            Ok(output) => {
                if !output.status.success() {
                    error!(
                        "unmount failed: {}",
                        String::from_utf8_lossy(&output.stderr)
                    );
                }
                info!(
                    "Unmounted {}: {}",
                    self.mountpoint.display(),
                    String::from_utf8_lossy(&output.stdout)
                );
            }
            Err(e) => {
                error!("Failed to execute unmount: {e}");
            }
        }
    }
}

pub struct Ssh {
    sess: Session,
}

impl Ssh {
    /// Create a new SSH session based on the provided configuration, performing host key
    /// verification and authentication.
    ///
    /// # Errors
    ///
    /// - issues connecting to the SSH server, performing the handshake, verifying the
    ///   host key, or authenticating (e.g., network errors, SSH errors, authentication failures)
    pub fn new(cfg: &config::MyConfig) -> anyhow::Result<Self> {
        // create ssh session
        let remote_user = &cfg.source.user;
        let known_hosts_path = PathBuf::from(&cfg.source.known_hosts);

        // Connect to the remote SSH server
        let tcp = TcpStream::connect(format!("{}:{}", &cfg.source.ip, &cfg.source.port))
            .with_context(|| {
                format!(
                    "Failed to connect to {}:{}",
                    &cfg.source.ip, &cfg.source.port
                )
            })?;
        let mut sess = Session::new().context("Failed to create SSH session")?;
        debug!(
            "Connected to SSH server at {}:{}",
            &cfg.source.ip, &cfg.source.port
        );
        sess.set_tcp_stream(tcp);
        debug!("Set TCP stream for SSH session");
        // Pin the handshake's host-key algorithm set to whatever is already in
        // known_hosts. Without this, libssh2 would default to its own
        // preference order (e.g. ecdsa-sha2-nistp256) and a phone that serves
        // multiple host keys would hand back a key of a different type than
        // the one we have pinned, producing a spurious Mismatch below. None
        // means "no entry yet" → skip the pin and trust whatever the server
        // serves on this first connect (TOFU; see block below).
        let algos = allowed_host_key_algos(&known_hosts_path, &cfg.source.ip, cfg.source.port)?;
        if let Some(ref a) = algos {
            debug!("Pinning host key algorithms from known_hosts: {a}");
            sess.method_pref(ssh2::MethodType::HostKey, a)
                .context("Failed to set host key algorithms")?;
        } else {
            debug!(
                "No known_hosts entry for {}:{} yet; letting libssh2 negotiate (TOFU on first connect)",
                &cfg.source.ip, cfg.source.port
            );
        }
        sess.handshake()
            .context("Failed to perform SSH handshake")?;
        debug!("SSH handshake completed");
        let (hostkey, key_type) = sess.host_key().context("Failed to get SSH host key")?;
        debug!("SSH host key type: {:?}", key_type);

        let mut known_hosts = sess
            .known_hosts()
            .context("Failed to get SSH known hosts")?;
        if known_hosts_path.exists() {
            known_hosts
                .read_file(&known_hosts_path, ssh2::KnownHostFileKind::OpenSSH)
                .with_context(|| format!("read known_hosts at {}", known_hosts_path.display()))?;
        }

        // NOT safe for untrusted networks/devices, but fine for a home LAN and avoids the need for a
        // separate "add host key" step before first run.
        match known_hosts.check_port(&cfg.source.ip, cfg.source.port, hostkey) {
            ssh2::CheckResult::Match => {
                debug!("SSH host key matches known hosts");
            }
            ssh2::CheckResult::Mismatch => {
                let actual = actual_fingerprint(&sess);
                let expected =
                    match expected_fingerprints(&known_hosts_path, &cfg.source.ip, cfg.source.port)
                    {
                        Ok(v) if v.is_empty() => "<no matching entry found>".to_string(),
                        Ok(v) => v.join("\n                          "),
                        Err(e) => format!("<failed to read: {e:#}>"),
                    };
                anyhow::bail!(
                    "SSH host key mismatch for {}:{}! Possible security issue.\n  \
                     expected (known_hosts): {}\n  \
                     actual   (server):      {}\n\
                     If you intentionally replaced the host key, remove the stale entry from {} and re-run.",
                    cfg.source.ip,
                    cfg.source.port,
                    expected,
                    actual,
                    known_hosts_path.display(),
                );
            }
            ssh2::CheckResult::NotFound => {
                let host_entry = if cfg.source.port == 22 {
                    cfg.source.ip.clone()
                } else {
                    format!("[{}]:{}", cfg.source.ip, cfg.source.port)
                };
                info!(
                    "SSH host key for {} not in known_hosts; trusting on first use and persisting to {}",
                    host_entry,
                    known_hosts_path.display()
                );
                known_hosts
                    .add(&host_entry, hostkey, "", key_type.into())
                    .context("failed to add host key to in-memory known_hosts")?;
                known_hosts
                    .write_file(&known_hosts_path, ssh2::KnownHostFileKind::OpenSSH)
                    .context("failed to persist host key to known_hosts file")?;
            }
            ssh2::CheckResult::Failure => {
                anyhow::bail!("Failed to check SSH host key against known hosts");
            }
        }

        // Authenticate using a certificate (public key)
        sess.userauth_pubkey_file(
            remote_user,
            None,
            PathBuf::from(&cfg.source.cert).as_path(),
            None,
        )
        .context("Failed to authenticate with SSH server using public key")?;

        // Check if authentication was successful
        if !sess.authenticated() {
            anyhow::bail!("SSH authentication failed");
        }
        Ok(Ssh { sess })
    }

    /// Execute a command on the remote SSH server and return its stdout as a String.
    ///
    /// # Errors
    ///
    /// - communication errors with phone (e.g., SSH errors)
    ///
    /// Note: no remote exit-status check - a failing remote command can return Ok("")
    pub fn execute_command(&self, cmd: &str) -> anyhow::Result<String> {
        let mut channel = self
            .sess
            .channel_session()
            .context("Failed to open SSH channel")?;
        channel
            .exec(cmd)
            // .exec("ls -l /sdcard/DCIM/Camera | awk '{print $8}'")
            .context(format!("Failed to execute command: {cmd}"))?;
        let mut s = String::new();
        channel
            .read_to_string(&mut s)
            .context("Failed to read command output")?;
        debug!("{}", &s);
        Ok(s)
    }
}
// Returns the comma-joined host-key algorithms found in known_hosts for this
// host, or None if there are no entries yet (first-ever run → caller should
// skip `method_pref` and let TOFU populate known_hosts after handshake).
fn allowed_host_key_algos(
    known_hosts: &Path,
    host: &str,
    port: u16,
) -> anyhow::Result<Option<String>> {
    if !known_hosts.exists() {
        return Ok(None);
    }
    let entries = lookup_known_hosts_entries(known_hosts, host, port)?;
    let mut algos: Vec<String> = Vec::new();
    for (algo, _) in entries {
        if !algos.contains(&algo) {
            algos.push(algo);
        }
    }
    if algos.is_empty() {
        Ok(None)
    } else {
        Ok(Some(algos.join(",")))
    }
}

// OpenSSH-style fingerprint of the live server key, e.g.
// `SHA256:abc… (ED25519)`.
fn actual_fingerprint(sess: &Session) -> String {
    use base64::Engine;
    let label = sess.host_key().map_or("unknown", |(_, ty)| {
        normalize_algo_label(host_key_type_to_algo(ty))
    });
    match sess.host_key_hash(ssh2::HashType::Sha256) {
        Some(h) => format!(
            "SHA256:{} ({label})",
            base64::engine::general_purpose::STANDARD_NO_PAD.encode(h)
        ),
        None => format!("<unavailable> ({label})"),
    }
}

// SHA256 fingerprints of all known_hosts entries that match this host, in
// the order ssh-keygen returned them. Hashed entries (`HashKnownHosts yes`)
// are dehashed by ssh-keygen, so they're included transparently.
fn expected_fingerprints(known_hosts: &Path, host: &str, port: u16) -> anyhow::Result<Vec<String>> {
    use base64::Engine;
    use sha2::{Digest, Sha256};

    if !known_hosts.exists() {
        return Ok(Vec::new());
    }
    let entries = lookup_known_hosts_entries(known_hosts, host, port)?;
    let mut out = Vec::with_capacity(entries.len());
    for (algo, b64key) in entries {
        let key_bytes = base64::engine::general_purpose::STANDARD
            .decode(&b64key)
            .with_context(|| format!("decode base64 key from known_hosts entry ({algo})"))?;
        let hash = Sha256::digest(&key_bytes);
        out.push(format!(
            "SHA256:{} ({})",
            base64::engine::general_purpose::STANDARD_NO_PAD.encode(hash),
            normalize_algo_label(&algo),
        ));
    }
    Ok(out)
}

// Look up entries via `ssh-keygen -F`, trying [host]:port then bare host.
// ssh-keygen handles plaintext, hashed, wildcard, and marker entries — we
// just take what it gives us.
fn lookup_known_hosts_entries(
    known_hosts: &Path,
    host: &str,
    port: u16,
) -> anyhow::Result<Vec<(String, String)>> {
    let mut entries = run_keygen_lookup(known_hosts, &format!("[{host}]:{port}"))?;
    if entries.is_empty() {
        entries = run_keygen_lookup(known_hosts, host)?;
    }
    Ok(entries)
}

fn run_keygen_lookup(known_hosts: &Path, pattern: &str) -> anyhow::Result<Vec<(String, String)>> {
    let out = Command::new("ssh-keygen")
        .args(["-F", pattern, "-f"])
        .arg(known_hosts)
        .output()
        .context("failed to run ssh-keygen (is openssh-client installed?)")?;

    // ssh-keygen -F: exit 0 = found, 1 = not found, other = error.
    match out.status.code() {
        Some(0) => {}
        Some(1) => return Ok(Vec::new()),
        _ => anyhow::bail!(
            "ssh-keygen -F {pattern} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ),
    }
    Ok(parse_keygen_lookup(&String::from_utf8_lossy(&out.stdout)))
}

// Parse `ssh-keygen -F` stdout into (algo, base64_key) pairs. Skips comments,
// blank lines, and `@cert-authority` / `@revoked` markers — neither is a
// usable host fingerprint to compare against. Hashed host patterns
// (`|1|salt|hash`) are kept: ssh-keygen already filtered to ours, so the
// pattern field is ignored and we read fields 2 (algo) and 3 (key).
fn parse_keygen_lookup(stdout: &str) -> Vec<(String, String)> {
    let mut entries = Vec::new();
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let Some(first) = parts.next() else { continue };
        if first.starts_with('@') {
            continue;
        }
        let Some(algo) = parts.next() else { continue };
        let Some(b64key) = parts.next() else { continue };
        entries.push((algo.to_string(), b64key.to_string()));
    }
    entries
}

// Map an OpenSSH host-key algorithm name to its short parenthesized label
// (RSA / DSA / ECDSA / ED25519), matching `ssh-keygen -l` output. Unknown
// algorithm strings are passed through unchanged.
fn normalize_algo_label(algo: &str) -> &str {
    match algo {
        "ssh-rsa" => "RSA",
        "ssh-dss" => "DSA",
        "ecdsa-sha2-nistp256" | "ecdsa-sha2-nistp384" | "ecdsa-sha2-nistp521" => "ECDSA",
        "ssh-ed25519" => "ED25519",
        other => other,
    }
}

fn host_key_type_to_algo(ty: ssh2::HostKeyType) -> &'static str {
    match ty {
        ssh2::HostKeyType::Rsa => "ssh-rsa",
        ssh2::HostKeyType::Dss => "ssh-dss",
        ssh2::HostKeyType::Ecdsa256 => "ecdsa-sha2-nistp256",
        ssh2::HostKeyType::Ecdsa384 => "ecdsa-sha2-nistp384",
        ssh2::HostKeyType::Ecdsa521 => "ecdsa-sha2-nistp521",
        ssh2::HostKeyType::Ed25519 => "ssh-ed25519",
        ssh2::HostKeyType::Unknown => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_algo_label_known() {
        assert_eq!(normalize_algo_label("ssh-rsa"), "RSA");
        assert_eq!(normalize_algo_label("ssh-dss"), "DSA");
        assert_eq!(normalize_algo_label("ssh-ed25519"), "ED25519");
        assert_eq!(normalize_algo_label("ecdsa-sha2-nistp256"), "ECDSA");
        assert_eq!(normalize_algo_label("ecdsa-sha2-nistp384"), "ECDSA");
        assert_eq!(normalize_algo_label("ecdsa-sha2-nistp521"), "ECDSA");
    }

    #[test]
    fn normalize_algo_label_passthrough() {
        assert_eq!(
            normalize_algo_label("ssh-rsa-cert-v01@openssh.com"),
            "ssh-rsa-cert-v01@openssh.com"
        );
        assert_eq!(normalize_algo_label(""), "");
    }

    #[test]
    fn host_key_type_to_algo_maps_all() {
        assert_eq!(host_key_type_to_algo(ssh2::HostKeyType::Rsa), "ssh-rsa");
        assert_eq!(host_key_type_to_algo(ssh2::HostKeyType::Dss), "ssh-dss");
        assert_eq!(
            host_key_type_to_algo(ssh2::HostKeyType::Ecdsa256),
            "ecdsa-sha2-nistp256"
        );
        assert_eq!(
            host_key_type_to_algo(ssh2::HostKeyType::Ecdsa384),
            "ecdsa-sha2-nistp384"
        );
        assert_eq!(
            host_key_type_to_algo(ssh2::HostKeyType::Ecdsa521),
            "ecdsa-sha2-nistp521"
        );
        assert_eq!(
            host_key_type_to_algo(ssh2::HostKeyType::Ed25519),
            "ssh-ed25519"
        );
        assert_eq!(host_key_type_to_algo(ssh2::HostKeyType::Unknown), "unknown");
    }

    #[test]
    fn parse_keygen_lookup_single_entry() {
        let stdout = "\
# Host 192.168.1.10 found: line 5 type ED25519
[192.168.1.10]:2222 ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIabc
";
        let entries = parse_keygen_lookup(stdout);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, "ssh-ed25519");
        assert_eq!(entries[0].1, "AAAAC3NzaC1lZDI1NTE5AAAAIabc");
    }

    #[test]
    fn parse_keygen_lookup_multiple_entries() {
        let stdout = "\
# Host 1.2.3.4 found: line 1 type ED25519
1.2.3.4 ssh-ed25519 AAAAaaa
# Host 1.2.3.4 found: line 2 type ECDSA
1.2.3.4 ecdsa-sha2-nistp256 AAAAbbb
";
        let entries = parse_keygen_lookup(stdout);
        assert_eq!(
            entries,
            vec![
                ("ssh-ed25519".to_string(), "AAAAaaa".to_string()),
                ("ecdsa-sha2-nistp256".to_string(), "AAAAbbb".to_string()),
            ]
        );
    }

    #[test]
    fn parse_keygen_lookup_hashed_pattern() {
        // Hashed entries: ssh-keygen -F has already matched, so we ignore the
        // pattern and read algo + key from fields 2 and 3.
        let stdout = "\
# Host 1.2.3.4 found: line 7
|1|abcdefghi=|hashashashash= ssh-rsa AAAArsa
";
        let entries = parse_keygen_lookup(stdout);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, "ssh-rsa");
        assert_eq!(entries[0].1, "AAAArsa");
    }

    #[test]
    fn parse_keygen_lookup_skips_markers() {
        let stdout = "\
@cert-authority *.example.com ssh-ed25519 AAAAcaca
@revoked 1.2.3.4 ssh-rsa AAAArevoked
1.2.3.4 ssh-ed25519 AAAAvalid
";
        let entries = parse_keygen_lookup(stdout);
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0],
            ("ssh-ed25519".to_string(), "AAAAvalid".to_string())
        );
    }

    #[test]
    fn parse_keygen_lookup_skips_blank_and_comment() {
        let stdout = "# leading comment\n\n1.2.3.4 ssh-ed25519 AAAAxx\n# trailing comment\n";
        let entries = parse_keygen_lookup(stdout);
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn parse_keygen_lookup_skips_truncated_lines() {
        let stdout = "\
1.2.3.4
1.2.3.4 ssh-ed25519
";
        assert!(parse_keygen_lookup(stdout).is_empty());
    }

    #[test]
    fn parse_keygen_lookup_empty_stdout() {
        assert!(parse_keygen_lookup("").is_empty());
    }

    #[test]
    fn plain() {
        assert_eq!(shell_escape("foo"), "'foo'");
    }

    #[test]
    fn empty() {
        assert_eq!(shell_escape(""), "''");
    }

    #[test]
    fn embedded_quote() {
        assert_eq!(shell_escape("a'b"), "'a'\\''b'");
    }

    #[test]
    fn injection_attempt() {
        // The classic: `; rm -rf /`. After escape, the result is one literal
        // argument — no command separator escapes the quotes.
        let s = shell_escape("2024-01-01; rm -rf /");
        assert_eq!(s, "'2024-01-01; rm -rf /'");
    }

    #[test]
    fn newline_and_specials() {
        assert_eq!(shell_escape("a\nb $c `d`"), "'a\nb $c `d`'");
    }
}
