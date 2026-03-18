use java_dataset_converter_llm::cli::Args;
use java_dataset_converter_llm::helper::get_files;
use java_dataset_converter_llm::obfuscator::obfuscate_str;
use java_dataset_converter_llm::processor::generate_jsonl_from_strings;
use java_dataset_converter_llm::sanitizer::sanitize_structural;

use clap::Parser;
use indicatif::{ProgressBar, ProgressStyle};
use rayon::prelude::*;
use std::fs;
use std::fs::OpenOptions;
use std::io;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

fn is_processed(java_file: &Path, jsonl_output_dir: &Path) -> bool {
    let file_name = java_file.file_name().unwrap().to_str().unwrap();
    jsonl_output_dir
        .join(format!("{}.jsonl", file_name))
        .exists()
}

fn log_error(log_path: &Path, java_file: &Path, stage: &str, err: &dyn std::error::Error) {
    // Mutex-guard the log file so parallel threads don't interleave writes.
    static LOG_LOCK: std::sync::OnceLock<Mutex<()>> = std::sync::OnceLock::new();
    let _guard = LOG_LOCK.get_or_init(|| Mutex::new(())).lock();

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

/// Normalise a raw `.java` source string.
///
/// Only structural fixes are applied here (JSON unicode escapes, `\'` → `'`,
/// CRLF → LF, null bytes).  Backslash normalisation is intentionally NOT done
/// here: `sanitize_backslashes` is a blind text replace that cannot distinguish
/// valid Java `"\\\\"` (a string whose value is `\\`) from the corrupted form
/// `"\\"` produced by over-encoding.  Applying it here would destroy valid
/// sources.  Both sides of the JSONL pair (`sanitized_original` and the output
/// of `obfuscate_str`) go through `sanitize_structural` only, so their string
/// literal structure is always identical and `fix_string_literals` never
/// produces a count mismatch.
fn full_sanitize(raw: &str) -> String {
    sanitize_structural(raw)
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
    progress_bar.set_style(
        ProgressStyle::with_template(
            "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta}) {msg}",
        )
        .unwrap()
        .progress_chars("#>-"),
    );
    progress_bar.set_message("Processing Java files...");
    progress_bar.inc(already_processed as u64);

    // ── Parallel processing with rayon ──────────────────────────────────────
    // Each file is fully independent (read → sanitize → obfuscate → write),
    // so we can safely process them in parallel. ProgressBar is Arc-based
    // internally and safe to share across threads.
    java_files
        .par_iter()
        .filter(|f| !is_processed(f, &jsonl_output_dir))
        .for_each(|file| {
            let file_name = file.file_name().unwrap().to_str().unwrap();

            // ── 1. Read & sanitize ────────────────────────────────────────────
            let raw = match fs::read_to_string(file) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("Error reading {}: {}", file_name, e);
                    log_error(&error_log_path, file, "read", &e);
                    progress_bar.inc(1);
                    return;
                }
            };
            let sanitized_original = full_sanitize(&raw);

            // ── 2. Obfuscate in memory ────────────────────────────────────────
            let obfuscated = match obfuscate_str(&sanitized_original) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("Error obfuscating {}: {}", file_name, e);
                    log_error(&error_log_path, file, "obfuscate", &e);
                    progress_bar.inc(1);
                    return;
                }
            };

            // ── 3. Write JSONL ────────────────────────────────────────────────
            let jsonl_file = jsonl_output_dir.join(format!("{}.jsonl", file_name));
            if let Err(e) = generate_jsonl_from_strings(
                &sanitized_original,
                &obfuscated,
                jsonl_file.to_str().unwrap(),
            ) {
                eprintln!("Error generating JSONL for {}: {}", file_name, e);
                log_error(&error_log_path, file, "generate_jsonl", &e);
            }

            progress_bar.inc(1);
        });

    progress_bar.finish_with_message("Done.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::full_sanitize;
    #[test]
    fn full_sanitize_does_not_alter_valid_backslash_pairs() {
        // Valid Java: string whose value is two backslashes.
        // full_sanitize must leave this byte-for-byte identical.
        let raw = r#"assertThat(result).isEqualTo("\\\\");"#;
        let result = full_sanitize(raw);
        assert_eq!(
            result, raw,
            "full_sanitize must not alter a valid \\\\\\\\ string literal"
        );
    }

    /// `full_sanitize` must still fix `\\'` → `'` (escaped apostrophe artifact).
    #[test]
    fn full_sanitize_fixes_escaped_apostrophe() {
        let raw = "Arrays.fill(buf, (byte) \\'Q\\');";
        let result = full_sanitize(raw);
        assert!(
            result.contains("'Q'"),
            "full_sanitize must convert \\' to '"
        );
        assert!(
            !result.contains("\\'"),
            "full_sanitize must remove all \\' sequences"
        );
    }
}
