use std::fs;
use std::io;
use std::path::{PathBuf};

pub fn get_files(dir: &str, extension: &str) -> io::Result<Vec<PathBuf>> {
    let mut matching_files = Vec::new();

    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.is_file() {
            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                if ext.eq_ignore_ascii_case(extension) {
                    matching_files.push(path);
                }
            }
        }
    }

    Ok(matching_files)
}
