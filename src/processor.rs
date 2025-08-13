// fn read_java_files(dir: &Path) -> Vec<(String, String)> {
//     WalkDir::new(dir)
//         .into_iter()
//         .filter_map(Result::ok)
//         .filter(|e| e.path().extension().map_or(false, |ext| ext == "java"))
//         .filter_map(|entry| {
//             let content = fs::read_to_string(entry.path()).ok()?;
//             let file_name = entry.path().file_name()?.to_string_lossy().to_string();
//             Some((file_name, content))
//         })
//         .collect()
// }

// fn generate_jsonl(
//     original_dir: &Path,
//     decompiled_dir: &Path,
//     output_file: &str,
// ) -> std::io::Result<()> {
//     let originals = read_java_files(original_dir);
//     let obfuscated = read_java_files(decompiled_dir);
//     let mut writer = BufWriter::new(File::create(output_file)?);

//     for (file_name, obf_code) in obfuscated {
//         if let Some((_, orig_code)) = originals.iter().find(|(name, _)| *name == file_name) {
//             let pair = PromptResponse {
//                 prompt: obf_code.clone(),
//                 response: orig_code.clone(),
//             };
//             let json_line = serde_json::to_string(&pair)?;
//             writeln!(writer, "{}", json_line)?;
//         }
//     }

//     println!("✅ JSONL dataset written to: {}", output_file);
//     Ok(())
// }


