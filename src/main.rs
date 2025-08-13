use java_dataset_converter_llm::obfuscator::obfuscate;
use java_dataset_converter_llm::cli::Args;
use java_dataset_converter_llm::helper::get_files;

use clap::Parser;
use std::fs;
use std::io;
use std::path::{Path};
use indicatif::ProgressBar;


fn main() -> io::Result<()> {
    let args = Args::parse();
    let input_dir = Path::new(&args.input);
    let output_dir = Path::new(&args.output);

    if !input_dir.exists() {
        eprintln!("Input directory does not exist: {}", input_dir.display());
        return Err(io::Error::new(io::ErrorKind::NotFound, "Input directory not found"));
    }

    if !output_dir.exists() {
        fs::create_dir_all(output_dir)?;
    }

    let java_files = get_files(input_dir.to_str().unwrap(), "java")?;

    let progress_bar = ProgressBar::new(java_files.len() as u64);

    for file in java_files {
        let file_name = file.file_name().unwrap().to_str().unwrap();
        let output_file = output_dir.join(file_name);

        if output_file.exists() {
            // Skip files that already exist in the output directory, a means of pause and stop for bigger datasets
            eprintln!("Skipping {} as it already exists in the output directory.", file_name);
        } else {
            match obfuscate(file.to_str().unwrap(), output_file.to_str().unwrap()) {
                Err(e) => eprintln!("Error obfuscating {}: {}", file_name, e),
                _ => {}
            }
        }

        progress_bar.inc(1);
    }

    progress_bar.finish_with_message("Obfuscation complete.");

    Ok(())
}
