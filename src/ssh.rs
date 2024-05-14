use crate::config;
use ssh2::Session;
use std::io::{Cursor, Read, Seek, SeekFrom};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct Ssh {
    sess: Session,
}
impl Ssh {
    pub fn new(cfg: &config::MyConfig) -> Self {
        // create ssh session
        let remote_user = "dummy";
        let private_key_path = Path::new(&cfg.source.cert);

        // mount sshfs
        let path = cfg.source.ip.to_string() + ":" + &cfg.source.path.to_string(); // Replace with your remote path

        let output = Command::new("sshfs")
            .arg("-p")
            .arg(&cfg.source.port.to_string())
            .arg(path)
            .arg(&cfg.general.mountpoint)
            .output()
            .expect("Failed to execute command");

        if !output.status.success() {
            eprintln!(
                "Command executed with error: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        } else {
            println!(
                "Command executed successfully: {}",
                String::from_utf8_lossy(&output.stdout)
            );
        }

        // Connect to the remote SSH server
        let tcp = TcpStream::connect(format!("{}:{}", &cfg.source.ip, &cfg.source.port)).unwrap();
        let mut sess = Session::new().unwrap();
        sess.set_tcp_stream(tcp);
        sess.handshake().unwrap();

        // Authenticate using a certificate (public key)
        sess.userauth_pubkey_file(
            remote_user,
            None,
            PathBuf::from(&cfg.source.cert).as_path(),
            None,
        )
        .unwrap();

        // Check if authentication was successful
        assert!(sess.authenticated());
        Ssh { sess }
    }
    pub fn execute_command(&self, cmd: String) -> String {
        let mut channel = self.sess.channel_session().unwrap();
        channel
            .exec(&cmd)
            // .exec("ls -l /sdcard/DCIM/Camera | awk '{print $8}'")
            .unwrap();
        let mut s = String::new();
        channel.read_to_string(&mut s).unwrap();
        println!("{}", &s);
        s
    }
}
