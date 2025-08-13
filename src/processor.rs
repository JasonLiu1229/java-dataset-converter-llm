use std::fs::File;
use std::io::{BufWriter, Write};
use serde::Serialize;
use std::fs;

#[derive(Serialize)]
struct PromptResponse {
    prompt: String,
    response: String,
}

pub fn generate_jsonL(
    original_file: &str,
    obfuscated_file: &str,
    output_file: &str,
) -> std::io::Result<()> {
    let original_code = fs::read_to_string(original_file)?;
    let obfuscated_code = fs::read_to_string(obfuscated_file)?;

    let mut writer = BufWriter::new(File::create(output_file)?);
    let pair = PromptResponse {
        prompt: obfuscated_code,
        response: original_code,
    };
    let json_line = serde_json::to_string(&pair)?;
    writeln!(writer, "{}", json_line)?;

    Ok(())
}
