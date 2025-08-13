use regex::Regex;
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::io;

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn obfuscate_code(input: &str) -> String {
    let java_keywords: [&'static str; 53] = [
        "abstract", "assert", "boolean", "break", "byte", "case", "catch", "char", "class",
        "const", "continue", "default", "do", "double", "else", "enum", "extends", "final",
        "finally", "float", "for", "goto", "if", "implements", "import", "instanceof", "int",
        "interface", "long", "native", "new", "null", "package", "private", "protected",
        "public", "return", "short", "static", "strictfp", "super", "switch", "synchronized",
        "this", "throw", "throws", "transient", "try", "void", "volatile", "while", "true",
        "false",
    ];

    let re = Regex::new(r"\b([a-zA-Z_][a-zA-Z0-9_]*(?:<[^>]+>)?(?:\[\s*\])?)\s+([a-zA-Z_][a-zA-Z0-9_]*)(\[\s*\])?").unwrap();
    let mut replacements = Vec::new();

    let result = re.replace_all(input, |caps: &regex::Captures| {
        let type_or_keyword = &caps[1];
        let identifier = &caps[2];
        let array_suffix = caps.get(3).map_or("", |m| m.as_str());

        if java_keywords.contains(&type_or_keyword) || java_keywords.contains(&identifier) {
            caps[0].to_string() // Skip Java keywords
        } else {
            let unique_id = COUNTER.fetch_add(1, Ordering::SeqCst);
            let replacement = format!("{} var_{}{}", type_or_keyword, unique_id, array_suffix);
            replacements.push((identifier.to_string(), format!("var_{}", unique_id)));
            replacement
        }
    }).to_string();

    apply_replacements(&result, &replacements)
}

fn obfuscate_function_names(java_code: &str) -> String {
    let re = Regex::new(
        r"(?m)(\b(?:public|private|protected)\s+(?:final\s+|static\s+|synchronized\s+|abstract\s+|native\s+|strictfp\s+)*[A-Za-z0-9_<>\[\].]+\s+)([A-Za-z_][A-Za-z0-9_]*)\s*(\([^)]*\)(?:\s*throws\s+[A-Za-z0-9_.,\s]+)?\s*\{)"
    ).unwrap();

    let mut counter = 0;
    re.replace_all(java_code, |caps: &regex::Captures| {
        counter += 1;
        format!("{}func_{}{}", &caps[1], counter, &caps[3])
    }).to_string()
}

fn apply_replacements(input: &str, replacements: &[(String, String)]) -> String {
    let mut result = input.to_string();
    for (original, replacement) in replacements {
        let re = Regex::new(&format!(r"\b{}\b", regex::escape(original))).unwrap();
        result = re
            .replace_all(&result, |caps: &regex::Captures| {
                let match_str = &caps[0];
                if !match_str.starts_with('.') && !result[caps.get(0).unwrap().end()..].starts_with('(') {
                    replacement.clone()
                } else {
                    match_str.to_string()
                }
            })
            .to_string();
    }
    result
}

pub fn obfuscate(input_file: &str, output_file: &str) -> io::Result<()> {
    let code = fs::read_to_string(input_file)?;
    let func_name_obfuscated = obfuscate_function_names(&code);
    let obfuscated = obfuscate_code(&func_name_obfuscated);
    fs::write(output_file, obfuscated)?;
    Ok(())
}
