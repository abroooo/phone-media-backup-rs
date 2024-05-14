use crate::helpers;
use std::fs::File;
use std::io::{Cursor, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

pub enum CompareMode {
    Lazy,
    Strict,
}

pub fn compare_files(
    source: &str,
    destination: &str,
    mode: CompareMode,
) -> std::io::Result<bool> {
    match mode {
        CompareMode::Lazy => {
            println!("lazy check");
            if Path::new(&destination).exists() {
                println!("files are identical");
                return Ok(true);
            }
            Ok(false)
        }
        CompareMode::Strict => {
            println!("strict check");
            if Path::new(&destination).exists() {
                return Ok(false);
            }
            let mut file1 = File::open(source)?;
            let mut file2 = File::open(destination)?;

            let mut buffer1 = [0; 4096];
            let mut buffer2 = [0; 4096];

            loop {
                let bytes_read1 = file1.read(&mut buffer1)?;
                let bytes_read2 = file2.read(&mut buffer2)?;

                if bytes_read1 != bytes_read2
                    || buffer1[..bytes_read1] != buffer2[..bytes_read2]
                {
                    return Ok(false);
                }

                if bytes_read1 == 0 {
                    break;
                }
            }
            println!("files are identical");

            Ok(true)
        }
    }
}
pub fn move_file(dloc: crate::helpers::DateLoc) {
    let destination_base = &dloc.destination_base.as_str();
    let dest_dir = [
        destination_base.to_string(),
        dloc.year.to_string(),
        dloc.month.to_string(),
    ]
    .join("/");
    let dest_file = [
        destination_base.to_string(),
        dloc.year.to_string(),
        dloc.month.to_string(),
        dloc.base_name,
    ]
    .join("/");
    let _ = std::fs::create_dir_all(&dest_dir);

    // println!("{}", filename);
    if compare_files(&dloc.source, &dest_file, CompareMode::Lazy).unwrap() {
        println!("exists!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!1");
    } else {
        let _ = std::fs::copy(&dloc.source, &dest_file);
        let _ = compare_files(&dloc.source, &dest_file, CompareMode::Strict);
        // let source_md5 = calculate_md5(&dloc.source);
        // let dest_md5 = calculate_md5(&dest_file);
        //   if source_md5 == dest_md5 {
        //     println!("Files are identical.");
        // } else {
        //     println!("Files are different.");
        // }
        let metadata = std::fs::metadata(&dloc.source).unwrap();
        let _ = helpers::set_file_times(&dest_file, metadata);
        println!("move {} to {}", dloc.source, dest_file);
    }
}
