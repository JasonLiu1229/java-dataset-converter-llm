use regex::Regex;
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::io;

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn obfuscate_java(input: &str) -> String {
    let re = Regex::new(r"\b(int|String|boolean|double|float|char|long|short|byte)\s+([a-zA-Z_][a-zA-Z0-9_]*)").unwrap();
    re.replace_all(input, |caps: &regex::Captures| {
        let unique_id = COUNTER.fetch_add(1, Ordering::SeqCst);
        format!("{} var_{}", &caps[1], unique_id)
    }).to_string()
}

fn main() -> io::Result<()> {
    let input_file = "Main.java";
    let output_file = "Main_obfuscated.java";

    let code = fs::read_to_string(input_file)?;
    let obfuscated = obfuscate_java(&code);
    fs::write(output_file, obfuscated)?;

    println!("Obfuscation complete. Output written to {}", output_file);
    Ok(())
}
