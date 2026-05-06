use crate::config;
use anyhow::Context;
use tracing::{debug, error, info};
use ssh2::Session;
use std::io::Read;
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::Command;

pub struct Mount {
    mountpoint: PathBuf,
}
impl Mount {
    pub fn new(cfg: &config::MyConfig) -> anyhow::Result<Self> {
        let out = Command::new("sshfs")
            .arg("-p")
            .arg(cfg.source.port.to_string())
            .arg(cfg.source.ip.clone() + ":" + &cfg.source.path.clone())
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
    pub fn new(cfg: &config::MyConfig) -> anyhow::Result<Self> {
        // create ssh session
        let remote_user = "dummy";
        // let private_key_path = Path::new(&cfg.source.cert);

        // Connect to the remote SSH server
        let tcp = TcpStream::connect(format!("{}:{}", &cfg.source.ip, &cfg.source.port))
            .with_context(|| {
                format!(
                    "Failed to connect to {}:{}",
                    &cfg.source.ip, &cfg.source.port
                )
            })?;
        let mut sess = Session::new().context("Failed to create SSH session")?;
        sess.set_tcp_stream(tcp);
        sess.handshake()
            .context("Failed to perform SSH handshake")?;

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
