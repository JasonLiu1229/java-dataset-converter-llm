use std::process::Command;
use std::path::{Path, PathBuf};
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use walkdir::WalkDir;
use serde::Serialize;

#[derive(Serialize)]
struct PromptResponse {
    prompt: String,
    response: String,
}

fn run_command(cmd: &mut Command) {
    println!(">> Running: {:?}", cmd);
    let status = cmd.status().expect("Failed to run command");
    if !status.success() {
        panic!("Command failed: {:?}", cmd);
    }
}

fn compile_java(train_src_dir: &str, validation_src_dir: &str, test_src_dir: &str, output_dir: &str) {
    // create output directory if it doesn't exist
    fs::create_dir_all(output_dir).unwrap();

    // create subdirectories for train, validation, and test in output_dir
    let train_output = PathBuf::from(output_dir).join("train");
    let validation_output = PathBuf::from(output_dir).join("validation");
    let test_output = PathBuf::from(output_dir).join("test");

    fs::create_dir_all(&train_output).unwrap();
    fs::create_dir_all(&validation_output).unwrap();
    fs::create_dir_all(&test_output).unwrap();

    // run javac for each source directory
    for src_dir in [train_src_dir, validation_src_dir, test_src_dir] {
        let mut cmd = Command::new("javac");
        cmd.arg("-d").arg(output_dir);
        cmd.arg("-sourcepath").arg(src_dir);
        for entry in WalkDir::new(src_dir)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|e| e.path().extension().map_or(false, |ext| ext
                == "java"))
        {
            cmd.arg(entry.path());
        }
        run_command(&mut cmd);
    }
}

fn create_jar(input_classes_dir: &str, output_jar: &str) {
    let train_output = PathBuf::from(input_classes_dir).join("train");
    let validation_output = PathBuf::from(input_classes_dir).join("validation");
    let test_output = PathBuf::from(input_classes_dir).join("test");

    for src_dir in [train_output, validation_output, test_output] {
        let mut cmd = Command::new("jar");
        cmd.arg("cf").arg(output_jar).arg("-C").arg(src_dir);
        run_command(&mut cmd);
    }
}

fn run_proguard(proguard_bin_dir: &str, config_file: &str) {
    let proguard_bin = PathBuf::from(proguard_bin_dir);
    let config_file = PathBuf::from(config_file);
    if !proguard_bin.exists() {
        panic!("ProGuard binary directory does not exist: {}", proguard_bin.display());
    }
    if !config_file.exists() {
        panic!("ProGuard config file does not exist: {}", config_file.display());
    }

    // Set the ProGuard binary directory as the current working directory
    std::env::set_current_dir(&proguard_bin).expect("Failed to change directory to ProGuard bin");
    
    if cfg!(target_os = "windows") {
        // use bat file
        let mut cmd = Command::new("cmd");
        cmd.arg("/C").arg("proguard.bat");
        cmd.arg(config_file);
        run_command(&mut cmd);
    }
    else {
        // use sh file
        let mut cmd = Command::new("sh");
        cmd.arg("proguard.sh");
        cmd.arg(config_file);
        run_command(&mut cmd);
    }
}

fn run_cfr(cfr_jar: &str, input_jar: &str, output_dir: &str) {
    fs::create_dir_all(output_dir).unwrap();
    let mut cmd = Command::new("java");
    cmd.args([
        "-jar",
        cfr_jar,
        input_jar,
        "--outputdir",
        output_dir,
        "--silent",
    ]);
    run_command(&mut cmd);
}

fn read_java_files(dir: &Path) -> Vec<(String, String)> {
    WalkDir::new(dir)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().map_or(false, |ext| ext == "java"))
        .filter_map(|entry| {
            let content = fs::read_to_string(entry.path()).ok()?;
            let file_name = entry.path().file_name()?.to_string_lossy().to_string();
            Some((file_name, content))
        })
        .collect()
}

fn generate_jsonl(
    original_dir: &Path,
    decompiled_dir: &Path,
    output_file: &str,
) -> std::io::Result<()> {
    let originals = read_java_files(original_dir);
    let obfuscated = read_java_files(decompiled_dir);
    let mut writer = BufWriter::new(File::create(output_file)?);

    for (file_name, obf_code) in obfuscated {
        if let Some((_, orig_code)) = originals.iter().find(|(name, _)| *name == file_name) {
            let pair = PromptResponse {
                prompt: obf_code.clone(),
                response: orig_code.clone(),
            };
            let json_line = serde_json::to_string(&pair)?;
            writeln!(writer, "{}", json_line)?;
        }
    }

    println!("✅ JSONL dataset written to: {}", output_file);
    Ok(())
}

fn main() -> std::io::Result<()> {
    let paths = (
        "train_src",                // original train .java files
        "validation_src",           // original validation .java files
        "test_src",                 // original test .java files
        "target_classes",           // compiled .class output
        "input.jar",                // JAR from original .class files
        "tools/proguard-7.7.0/bin", // path to proguard bin files
        "proguard-project.pro",     // ProGuard config
        "obfuscated.jar",           // obfuscated output
        "cfr.jar",                  // path to cfr.jar
        "decompiled",               // decompiled obfuscated .java files
        "refactor_dataset.jsonl",   // final jsonl output
    );

    let (train_src_dir, validation_src_dir, test_src_dir, class_dir, jar_path, proguard_bin_dir, proguard_cfg, obf_jar, cfr_jar, decomp_dir, jsonl_out) = paths;

    compile_java(src_dir, class_dir);
    create_jar(class_dir, jar_path);
    run_proguard(proguard_bin_dir, proguard_cfg);
    run_cfr(cfr_jar, obf_jar, decomp_dir);
    generate_jsonl(Path::new(src_dir), Path::new(decomp_dir), jsonl_out)?;

    Ok(())
}
