
use crate::config;
use ssh2::Session;
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn get_session(cfg: &config::MyConfig) -> Session {
    // create ssh session
    let remote_user = "user";
    // let remote_host = "1.1.0.2";
    // let remote_user = "your_username";
    // let private_key_path = "/path/to/your/private_key";
    let private_key_path = Path::new(&cfg.source.cert);

    // mount sshfs
    let port = "2222"; // Replace with your port number
    let path = cfg.source.ip.to_string() + ":/sdcard/DCIM/Camera"; // Replace with your remote path

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
    sess
}
