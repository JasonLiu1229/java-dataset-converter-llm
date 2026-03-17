use java_dataset_converter_llm::cli::Args;
use java_dataset_converter_llm::helper::get_files;
use java_dataset_converter_llm::obfuscator::obfuscate_str;
use java_dataset_converter_llm::processor::generate_jsonl_from_strings;
use java_dataset_converter_llm::sanitizer::sanitize_structural;

use clap::Parser;
use indicatif::ProgressBar;
use std::fs;
use std::fs::OpenOptions;
use std::io;
use std::io::Write;
use std::path::{Path, PathBuf};

fn is_processed(java_file: &Path, jsonl_output_dir: &Path) -> bool {
    let file_name = java_file.file_name().unwrap().to_str().unwrap();
    jsonl_output_dir
        .join(format!("{}.jsonl", file_name))
        .exists()
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
    let jsonl_output_dir = match &args.jsonl_output {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from(&args.output),
    };

    if !input_dir.exists() {
        eprintln!("Input directory does not exist: {}", input_dir.display());
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "Input directory not found",
        ));
    }

    fs::create_dir_all(&jsonl_output_dir)?;

    let error_log_path = jsonl_output_dir.join("error.log");
    let java_files = get_files(input_dir.to_str().unwrap(), "java")?;
    let total = java_files.len();

    let already_processed = java_files
        .iter()
        .filter(|f| is_processed(f, &jsonl_output_dir))
        .count();

    let progress_bar = ProgressBar::new(total as u64);
    progress_bar.set_message("Processing Java files...");
    progress_bar.inc(already_processed as u64);

    for file in java_files {
        if is_processed(&file, &jsonl_output_dir) {
            continue;
        }

        let file_name = file.file_name().unwrap().to_str().unwrap();

        // ── 1. Read & structural-sanitize ────────────────────────────────────
        let raw = match fs::read_to_string(&file) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Error reading {}: {}", file_name, e);
                log_error(&error_log_path, &file, "read", &e);
                progress_bar.inc(1);
                continue;
            }
        };
        let sanitized_original = sanitize_structural(&raw);

        // ── 2. Obfuscate in memory ────────────────────────────────────────────
        let obfuscated = match obfuscate_str(&sanitized_original) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Error obfuscating {}: {}", file_name, e);
                log_error(&error_log_path, &file, "obfuscate", &e);
                progress_bar.inc(1);
                continue;
            }
        };

        // ── 3. Write JSONL ────────────────────────────────────────────────────
        let jsonl_file = jsonl_output_dir.join(format!("{}.jsonl", file_name));
        if let Err(e) = generate_jsonl_from_strings(
            &sanitized_original,
            &obfuscated,
            jsonl_file.to_str().unwrap(),
        ) {
            eprintln!("Error generating JSONL for {}: {}", file_name, e);
            log_error(&error_log_path, &file, "generate_jsonl", &e);
        }

        progress_bar.inc(1);
    }

    progress_bar.finish_with_message("Done.");
    Ok(())
}
