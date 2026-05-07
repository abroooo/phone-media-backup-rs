use std::ffi::OsStr;
use std::path::Path;

/// Set the access and modification times of the destination file to match those of the source
/// file.
///
/// # Errors
///
/// - I/O errors
pub fn set_file_times(dest_file: &Path, metadata: &std::fs::Metadata) -> anyhow::Result<()> {
    filetime::set_file_times(
        dest_file,
        filetime::FileTime::from_last_access_time(metadata),
        filetime::FileTime::from_last_modification_time(metadata),
    )?;
    Ok(())
}

pub fn get_extension_from_filename(filename: &Path) -> Option<&str> {
    filename.extension().and_then(OsStr::to_str)
}
