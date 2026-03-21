use java_dataset_converter_llm::cli::Args;
use java_dataset_converter_llm::helper::get_files;
use java_dataset_converter_llm::obfuscator::obfuscate_str_checked;
use java_dataset_converter_llm::processor::{generate_jsonl_from_strings, generate_jsonl_raw};
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
    let clean = jsonl_output_dir
        .join(format!("{}.jsonl", file_name))
        .exists();
    let blanked_dir = blanked_subdir_of(jsonl_output_dir);
    let in_blanked = blanked_dir.join(format!("{}.jsonl", file_name)).exists();
    clean || in_blanked
}

fn blanked_subdir_of(jsonl_output_dir: &Path) -> PathBuf {
    let parent = jsonl_output_dir.parent().unwrap_or(Path::new("."));
    let dir_name = jsonl_output_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("jsonl");
    parent.join(format!("{}_blanked", dir_name))
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

    // Create the blanked subdir eagerly only when the feature is enabled.
    let jsonl_blanked_dir = if args.blanked_subdir {
        let d = blanked_subdir_of(&jsonl_output_dir);
        fs::create_dir_all(&d)?;
        Some(d)
    } else {
        None
    };

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

    java_files
        .par_iter()
        .filter(|f| !is_processed(f, &jsonl_output_dir))
        .for_each(|file| {
            let file_name = file.file_name().unwrap().to_str().unwrap();

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

            let (obfuscated, needed_fallback) = match obfuscate_str_checked(&sanitized_original) {
                Ok(pair) => pair,
                Err(e) => {
                    eprintln!("Error obfuscating {}: {}", file_name, e);
                    log_error(&error_log_path, file, "obfuscate", &e);
                    progress_bar.inc(1);
                    return;
                }
            };

            // ── 3. Route & write JSONL ────────────────────────────────────────
            if !needed_fallback {
                // Clean source: write with real string content preserved.
                let jsonl_file = jsonl_output_dir.join(format!("{}.jsonl", file_name));
                if let Err(e) = generate_jsonl_raw(
                    &sanitized_original,
                    &obfuscated,
                    jsonl_file.to_str().unwrap(),
                ) {
                    eprintln!("Error generating JSONL for {}: {}", file_name, e);
                    log_error(&error_log_path, file, "generate_jsonl", &e);
                }
            } else if let Some(ref blanked_dir) = jsonl_blanked_dir {
                // Corrupt source + --blanked-subdir: write blanked pair to sibling dir.
                let jsonl_file = blanked_dir.join(format!("{}.jsonl", file_name));
                if let Err(e) = generate_jsonl_from_strings(
                    &sanitized_original,
                    &obfuscated,
                    jsonl_file.to_str().unwrap(),
                ) {
                    eprintln!("Error generating blanked JSONL for {}: {}", file_name, e);
                    log_error(&error_log_path, file, "generate_jsonl_blanked", &e);
                }
            } else {
                // Corrupt source + no flag: skip silently (file stays unprocessed).
                eprintln!(
                    "Skipping {} (corrupt source, --blanked-subdir not set)",
                    file_name
                );
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
        let raw = r#"assertThat(result).isEqualTo("\\\\");"#;
        let result = full_sanitize(raw);
        assert_eq!(
            result, raw,
            "full_sanitize must not alter a valid \\\\\\\\ string literal"
        );
    }

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

    // ── blanked_subdir_of helper ─────────────────────────────────────────────

    use crate::blanked_subdir_of;
    use std::path::Path;

    #[test]
    fn blanked_subdir_of_appends_blanked_suffix() {
        let dir = Path::new("train/jsonl");
        let blanked = blanked_subdir_of(dir);
        assert_eq!(
            blanked,
            std::path::PathBuf::from("train/jsonl_blanked"),
            "sibling directory must be '<parent>/<name>_blanked'"
        );
    }

    #[test]
    fn blanked_subdir_of_nested_path() {
        let dir = Path::new("data/splits/test/jsonl");
        let blanked = blanked_subdir_of(dir);
        assert_eq!(
            blanked,
            std::path::PathBuf::from("data/splits/test/jsonl_blanked")
        );
    }

    #[test]
    fn blanked_subdir_of_top_level_name() {
        // When the input path has no parent component (e.g. "jsonl"), the
        // platform may return "" or "." for the parent, yielding either
        // "jsonl_blanked" or "./jsonl_blanked".  Both are equivalent paths;
        // assert on the file_name component only.
        let dir = Path::new("jsonl");
        let blanked = blanked_subdir_of(dir);
        assert_eq!(
            blanked.file_name().and_then(|n| n.to_str()),
            Some("jsonl_blanked"),
            "basename of the blanked sibling must be 'jsonl_blanked'"
        );
    }

    #[test]
    fn obfuscate_str_checked_clean_source_no_fallback() {
        use java_dataset_converter_llm::obfuscator::obfuscate_str_checked;

        let clean = concat!(
            "public class T {\n",
            "@Test public void testFoo() {",
            " String s = \"hello\"; assertEquals(\"hello\", s);",
            " }\n}",
        );
        let (result, needed_fallback) =
            obfuscate_str_checked(clean).expect("obfuscate_str_checked must not fail");

        assert!(
            !needed_fallback,
            "clean source must not trigger the literal-blanker fallback"
        );
        assert!(
            !result.contains("testFoo"),
            "method name must be renamed even without fallback"
        );
        // Real string content must be preserved — NOT replaced with "_".
        assert!(
            result.contains("\"hello\""),
            "string literal content must be preserved on the clean path, got: {result}"
        );
        assert!(
            !result.contains("\"_\""),
            "clean path must not produce dummy '_' literals, got: {result}"
        );
    }

    #[test]
    fn obfuscate_str_checked_corrupt_source_sets_fallback_flag() {
        use java_dataset_converter_llm::obfuscator::obfuscate_str_checked;

        // This is the TestClass10179 pattern: `\\n \\\"name\\\"` in the source
        // file contains `\\n` (two backslashes + 'n') followed by a space then
        // `\\"` (two backslashes + bare quote).  After blank_literals_permanently
        // the scanner sees an even backslash run followed by `"`, then checks the
        // next byte: a space is NOT in the suspicious set, so it closes the string
        // early — leaving raw tokens outside a string that tree-sitter flags as
        // ERROR nodes.  This is exactly the condition that must set needed_fallback=true.
        //
        // Contrast with `{\\\"key\\\"` where `{` IS suspicious: the heuristic
        // keeps the string open and no parse error fires.
        let corrupt = concat!(
            "public class T {\n",
            "@Test public void testCorrupt() throws Exception {",
            " HttpRequest var_req = client().GET(\"/api/test\");",
            // \\n = two backslashes + 'n' in value; space after = NOT suspicious → early close
            " assertResponse(var_req, 200, \"{\\\\n \\\\\"name\\\\\" : \\\\\"val\\\\\"\\\\n}\");",
            " }\n}",
        );
        let (result, needed_fallback) =
            obfuscate_str_checked(corrupt).expect("obfuscate_str_checked must not fail");

        assert!(
            needed_fallback,
            "TestClass10179-style source (\\\\n + space + \\\\\") must trigger \
             the literal-blanker fallback (needed_fallback=true)"
        );
        assert!(
            !result.contains("testCorrupt"),
            "method name must still be renamed after fallback"
        );
    }
}
