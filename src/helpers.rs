use std::ffi::OsStr;
use std::path::{Path, PathBuf};

pub fn set_file_times(dest_file: &PathBuf, metadata: &std::fs::Metadata) -> std::io::Result<()> {
    filetime::set_file_times(
        dest_file,
        filetime::FileTime::from_last_access_time(metadata),
        filetime::FileTime::from_last_modification_time(metadata),
    )?;
    Ok(())
}

pub fn get_extension_from_filename(filename: &PathBuf) -> Option<&str> {
    Path::new(filename).extension().and_then(OsStr::to_str)
}
