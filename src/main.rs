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

fn create_jar(input_classes_dir: &str) {
    let train_output = PathBuf::from(input_classes_dir).join("train");
    let validation_output = PathBuf::from(input_classes_dir).join("validation");
    let test_output = PathBuf::from(input_classes_dir).join("test");

    for src_dir in [train_output, validation_output, test_output] {
        let mut cmd = Command::new("jar");
        
        // Create the JAR file as <output_jar>_<type>.jar
        let type_datasets = ["train", "validation", "test"];
        for type_dataset in type_datasets {
            let jar_file = format!("input_{}.jar", type_dataset);
            cmd.arg("cf").arg(&jar_file);
            cmd.arg("-C").arg(&src_dir).arg(".");
            run_command(&mut cmd);
            println!("✅ Created JAR: {}", jar_file); 
        }
    }
}

fn run_proguard(proguard_bin_dir: &str, config_file: &str) {
    let proguard_bin = Path::new(proguard_bin_dir);
    let config_file = Path::new(config_file);
    if !proguard_bin.exists() {
        panic!("ProGuard binary directory does not exist: {}", proguard_bin.display());
    }
    if !config_file.exists() {
        panic!("ProGuard config file does not exist: {}", config_file.display());
    }

    std::env::set_current_dir(proguard_bin).expect("Failed to change directory to ProGuard bin");

    let type_datasets = ["train", "validation", "test"];
    if cfg!(target_os = "windows") {
        for type_dataset in type_datasets {
            let mut cmd = Command::new("cmd");
            cmd.args([
                "/C", "proguard.bat",
                "-injars", &format!("input_{}.jar", type_dataset),
                "-outjars", &format!("obfuscated_{}.jar", type_dataset),
                "-libraryjars", "<java.home>/lib/rt.jar",
                "-include", config_file.to_str().unwrap(),
            ]);
            run_command(&mut cmd);
        }
    } else {
        for type_dataset in type_datasets {
            let mut cmd = Command::new("sh");
            cmd.args([
                "proguard.sh",
                "-injars", &format!("input_{}.jar", type_dataset),
                "-outjars", &format!("obfuscated_{}.jar", type_dataset),
                "-libraryjars", "<java.home>/lib/rt.jar",
                "-include", config_file.to_str().unwrap(),
            ]);
            run_command(&mut cmd);
        }
    }
}

fn run_cfr(cfr_jar: &str, output_dir: &str) {
    let type_datasets = ["train", "validation", "test"];
    for type_dataset in type_datasets {
        let input_jar = format!("obfuscated_{}.jar", type_dataset);
        let output_dir = Path::new(output_dir).join(type_dataset);

        fs::create_dir_all(&output_dir).unwrap();

        let mut cmd = Command::new("java");
        cmd.args([
            "-jar",
            cfr_jar,
            &input_jar,
            "--outputdir",
            output_dir.to_str().unwrap(),
            "--silent",
        ]);
        run_command(&mut cmd);
        println!("✅ Decompiled {} dataset to: {}", type_dataset, output_dir.display());
    }
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
        "tools/proguard-7.7.0/bin", // path to proguard bin files
        "proguard-project.pro",     // ProGuard config
        "tools/cfr-0.152.jar",      // path to cfr.jar
        "decompiled",               // decompiled directory with train, validation, and test folders
        "original_dir",             // original directory with train, validation, and test folders
    );

    let (train_src_dir, validation_src_dir, test_src_dir, class_dir, proguard_bin_dir, proguard_cfg, obf_jar, cfr_jar, decomp_dir) = paths;

    compile_java(src_dir, class_dir);
    create_jar(class_dir);
    run_proguard(proguard_bin_dir, proguard_cfg);
    run_cfr(cfr_jar, decomp_dir);
    // generate_jsonl(Path::new(src_dir), Path::new(decomp_dir), jsonl_out)?;

    Ok(())
}
