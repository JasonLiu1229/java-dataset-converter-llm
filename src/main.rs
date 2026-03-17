use java_dataset_converter_llm::cli::Args;
use java_dataset_converter_llm::helper::get_files;
use java_dataset_converter_llm::obfuscator::obfuscate;
use java_dataset_converter_llm::processor::generate_jsonl;
use java_dataset_converter_llm::sanitizer::sanitize_structural;

use clap::Parser;
use indicatif::ProgressBar;
use std::fs;
use std::fs::OpenOptions;
use std::io;
use std::io::Write;
use std::path::{Path, PathBuf};

fn is_processed(
    java_file: &Path,
    output_dir: &Path,
    jsonl_output_dir: &Path,
    jsonl_enabled: bool,
) -> bool {
    let file_name = java_file.file_name().unwrap().to_str().unwrap();
    let obfuscated = output_dir.join(file_name);

    if !obfuscated.exists() {
        return false;
    }

    if jsonl_enabled {
        let jsonl = jsonl_output_dir.join(format!("{}.jsonl", file_name));
        jsonl.exists()
    } else {
        true
    }
}

fn log_error(log_path: &Path, java_file: &Path, stage: &str, err: &dyn std::error::Error) {
    let mut f = match OpenOptions::new().create(true).append(true).open(log_path) {
        Ok(f) => f,
        Err(_) => return,
    };

    let _ = writeln!(
        f,
        "[stage={}] file={} error={}",
        stage,
        java_file.display(),
        err
    );
}

fn main() -> io::Result<()> {
    let args = Args::parse();
    let input_dir = Path::new(&args.input);
    let output_dir = Path::new(&args.output);
    let jsonl_output_dir = match &args.jsonl_output {
        Some(dir) => Path::new(dir),
        None => output_dir,
    };

    if !input_dir.exists() {
        eprintln!("Input directory does not exist: {}", input_dir.display());
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "Input directory not found",
        ));
    }

    if !output_dir.exists() {
        fs::create_dir_all(output_dir)?;
    }

    if !jsonl_output_dir.exists() {
        fs::create_dir_all(jsonl_output_dir)?;
    }

    let error_log_path: PathBuf = output_dir.join("error.log");

    let java_files = get_files(input_dir.to_str().unwrap(), "java")?;
    let total = java_files.len();
    let jsonl_enabled = args.jsonl_output.is_some();

    let already_processed = java_files
        .iter()
        .filter(|file| is_processed(file, output_dir, jsonl_output_dir, jsonl_enabled))
        .count();

    let progress_bar = ProgressBar::new(total as u64);
    progress_bar.set_message("Processing Java files...");
    progress_bar.inc(already_processed as u64);

    for file in java_files {
        if is_processed(&file, output_dir, jsonl_output_dir, jsonl_enabled) {
            continue;
        }

        let file_name = file.file_name().unwrap().to_str().unwrap();
        let output_file = output_dir.join(file_name);

        // Sanitize the original source (structural pass only) before obfuscating.
        // This ensures tree-sitter sees clean input (resolved unicode escapes,
        // fixed single-quote escapes, normalised line endings) so identifier
        // boundaries are detected correctly.  We write the sanitized version to
        // a temp path and pass that to obfuscate() and generate_jsonl(), so all
        // three stages — sanitize, obfuscate, JSONL generation — operate on the
        // same byte sequence.
        let sanitized_original_path = output_dir.join(format!("{}.sanitized.java", file_name));

        if !output_file.exists() {
            // Write sanitized original so obfuscate() reads clean input.
            let raw = match fs::read_to_string(&file) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("Error reading {}: {}", file_name, e);
                    log_error(&error_log_path, &file, "read", &e);
                    progress_bar.inc(1);
                    continue;
                }
            };
            let sanitized = sanitize_structural(&raw);
            if let Err(e) = fs::write(&sanitized_original_path, &sanitized) {
                eprintln!("Error writing sanitized source for {}: {}", file_name, e);
                log_error(&error_log_path, &file, "sanitize_write", &e);
                progress_bar.inc(1);
                continue;
            }

            if let Err(e) = obfuscate(
                sanitized_original_path.to_str().unwrap(),
                output_file.to_str().unwrap(),
            ) {
                eprintln!("Error obfuscating {}: {}", file_name, e);
                log_error(&error_log_path, &file, "obfuscate", &e);
                // Clean up temp file on failure.
                let _ = fs::remove_file(&sanitized_original_path);
                progress_bar.inc(1);
                continue;
            }
        }

        if jsonl_enabled {
            let jsonl_file = jsonl_output_dir.join(format!("{}.jsonl", file_name));
            if !jsonl_file.exists() {
                // Use the sanitized original (not the raw file) so generate_jsonl
                // sees the same byte sequence the obfuscator worked on.
                let original_for_jsonl = if sanitized_original_path.exists() {
                    sanitized_original_path.clone()
                } else {
                    // Already processed in a previous run — re-sanitize on the fly.
                    let raw = fs::read_to_string(&file).unwrap_or_default();
                    let sanitized = sanitize_structural(&raw);
                    let _ = fs::write(&sanitized_original_path, &sanitized);
                    sanitized_original_path.clone()
                };

                if let Err(e) = generate_jsonl(
                    original_for_jsonl.to_str().unwrap(),
                    output_file.to_str().unwrap(),
                    jsonl_file.to_str().unwrap(),
                ) {
                    eprintln!("Error generating JSONL for {}: {}", file_name, e);
                    log_error(&error_log_path, &file, "generate_jsonl", &e);
                    // keep going
                }
            }
        }

        progress_bar.inc(1);
    }

    progress_bar.finish_with_message("Obfuscation complete.");
    Ok(())
}
