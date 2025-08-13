use regex::Regex;
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::io;

static COUNTER: AtomicUsize = AtomicUsize::new(0);

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

fn find_protected_ranges(code: &str) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut chars = code.char_indices().peekable();

    while let Some((i, c)) = chars.next() {
        match c {
            '"' => {
                let start = i;
                let mut escaped = false;
                while let Some((j, ch)) = chars.next() {
                    if ch == '\\' {
                        escaped = !escaped;
                    } else if ch == '"' && !escaped {
                        ranges.push((start, j + 1));
                        break;
                    } else {
                        escaped = false;
                    }
                }
            }
            '/' => {
                if let Some(&(_, '/')) = chars.peek() {
                    chars.next();
                    let start = i;
                    while let Some((j, ch)) = chars.next() {
                        if ch == '\n' {
                            ranges.push((start, j));
                            break;
                        }
                    }
                } else if let Some(&(_, '*')) = chars.peek() {
                    chars.next();
                    let start = i;
                    while let Some((j, ch)) = chars.next() {
                        if ch == '*' {
                            if let Some(&(_, '/')) = chars.peek() {
                                chars.next();
                                ranges.push((start, j + 2));
                                break;
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    ranges
}

fn is_in_ranges(pos: usize, ranges: &[(usize, usize)]) -> bool {
    ranges.iter().any(|&(s, e)| pos >= s && pos < e)
}

fn apply_replacements(input: &str, replacements: &[(String, String)], protected: &[(usize, usize)]) -> String {
    let mut result = input.to_string();
    for (original, replacement) in replacements {
        let re = Regex::new(&format!(r"(?m)(^|[^A-Za-z0-9_$])({})([^A-Za-z0-9_$]|$)", regex::escape(original))).unwrap();
        let snapshot = result.clone();
        result = re.replace_all(&snapshot, |caps: &regex::Captures| {
            let start = caps.get(2).unwrap().start();
            if is_in_ranges(start, protected) {
                return caps[0].to_string(); // don't replace inside protected areas
            }
            let before = caps.get(1).unwrap().as_str();
            let after = caps.get(3).unwrap().as_str();
            if after.trim_start().starts_with('(') {
                format!("{before}{}{}", original, after)
            } else {
                format!("{before}{}{}", replacement, after)
            }
        }).to_string();
    }
    result
}

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

    let protected = find_protected_ranges(input);

    let re = Regex::new(r"(?m)(?:(?:public|private|protected|static|final|volatile|transient)\s+)*([A-Za-z_][A-Za-z0-9_]*(?:<[^>]+>)?(?:\[\s*\])?)\s+([A-Za-z_][A-Za-z0-9_]*)(\[\s*\])?")
        .unwrap();

    let mut replacements = Vec::new();

    let result = re.replace_all(input, |caps: &regex::Captures| {
        let start = caps.get(0).unwrap().start();

        if is_in_ranges(start, &protected) {
            return caps[0].to_string(); 
        }
        let end = caps.get(0).unwrap().end();

        if input[end..].trim_start().starts_with('(') {
            return caps[0].to_string(); 
        }

        let type_or_kw = &caps[1];

        let ident = &caps[2];

        let array_suffix = caps.get(3).map_or("", |m| m.as_str());
        
        if java_keywords.contains(&type_or_kw) || java_keywords.contains(&ident) {
            caps[0].to_string()
        } else {
            let id = COUNTER.fetch_add(1, Ordering::SeqCst);
            replacements.push((ident.to_string(), format!("var_{}", id)));
            format!("{} var_{}{}", type_or_kw, id, array_suffix)
        }
    }).to_string();

    apply_replacements(&result, &replacements, &protected)
}

pub fn obfuscate(input_file: &str, output_file: &str) -> io::Result<()> {
    let code = fs::read_to_string(input_file)?;
    let func_name_obfuscated = obfuscate_function_names(&code);
    let obfuscated = obfuscate_code(&func_name_obfuscated);
    fs::write(output_file, obfuscated)?;
    Ok(())
}
