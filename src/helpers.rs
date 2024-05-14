use md5::{Digest, Md5};
use std::ffi::OsStr;
use std::fs::File;
use std::io::{Cursor, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

pub struct DateLoc {
    pub month: String,
    pub year: String,
    pub source: String,
    pub base_name: String,
    pub destination_base: String,
}
pub fn set_file_times(dest_file: &str, metadata: std::fs::Metadata) -> std::io::Result<()> {
    filetime::set_file_times(
        &dest_file,
        filetime::FileTime::from_last_access_time(&metadata),
        filetime::FileTime::from_last_modification_time(&metadata),
    )?;
    Ok(())
}
pub fn calculate_md5(filename: &str) -> Vec<u8> {
    let mut file = File::open(filename).expect("Unable to open file");
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer).expect("Unable to read file");
    let mut hasher = Md5::new();
    hasher.update(&buffer);
    hasher.finalize().to_vec()
}
pub fn get_extension_from_filename(filename: &str) -> Option<&str> {
    Path::new(filename).extension().and_then(OsStr::to_str)
}
