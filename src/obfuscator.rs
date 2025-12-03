use regex::Regex;
use std::fs;
use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};

use tree_sitter::{Node, Parser, TreeCursor};
use tree_sitter_java;

static COUNTER: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug, Clone)]
struct Replacement {
    start: usize,
    end: usize,
    text: String,
}

fn apply_replacements(source: &str, replacements: &[Replacement]) -> String {
    let mut result = source.to_string();
    let mut reps: Vec<_> = replacements.to_vec();
    reps.sort_by_key(|r| std::cmp::Reverse(r.start));

    for r in reps {
        result.replace_range(r.start..r.end, &r.text);
    }

    result
}

fn obfuscate_function_names(java_code: &str) -> String {
    let mut parser = Parser::new();
    let language = tree_sitter_java::LANGUAGE;
    parser
        .set_language(&language.into())
        .expect("Error loading Java grammar");

    let tree = match parser.parse(java_code, None) {
        Some(t) => t,
        None => return java_code.to_string(),
    };

    let root = tree.root_node();
    let mut cursor = root.walk();

    let mut replacements: Vec<Replacement> = Vec::new();
    let mut func_counter: usize = 1;

    fn walk(
        node: Node,
        cursor: &mut TreeCursor,
        func_counter: &mut usize,
        replacements: &mut Vec<Replacement>,
        source: &str,
    ) {
        if node.kind() == "method_declaration" {
            if let Some(name_node) = node.child_by_field_name("name") {
                let start = name_node.start_byte() as usize;
                let end = name_node.end_byte() as usize;
                let new_name = format!("func_{}", *func_counter);
                *func_counter += 1;

                // Optionally debug:
                // eprintln!("Renaming {} -> {}", &source[start..end], new_name);

                replacements.push(Replacement {
                    start,
                    end,
                    text: new_name,
                });
            }
        }

        if node.child_count() > 0 && cursor.goto_first_child() {
            loop {
                let child = cursor.node();
                walk(child, cursor, func_counter, replacements, source);
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
            cursor.goto_parent();
        }
    }

    walk(
        root,
        &mut cursor,
        &mut func_counter,
        &mut replacements,
        java_code,
    );

    apply_replacements(java_code, &replacements)
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

fn apply_replacements_vars(
    input: &str,
    replacements: &[(String, String)],
    protected: &[(usize, usize)],
) -> String {
    let mut result = input.to_string();
    for (original, replacement) in replacements {
        let re = Regex::new(&format!(r"(^|\W)({})(\W|$)", regex::escape(original))).unwrap();

        let snapshot = result.clone();
        result = re
            .replace_all(&snapshot, |caps: &regex::Captures| {
                let start = caps.get(2).unwrap().start();
                if is_in_ranges(start, protected) {
                    return caps[0].to_string();
                }
                format!(
                    "{}{}{}",
                    caps.get(1).unwrap().as_str(),
                    replacement,
                    caps.get(3).unwrap().as_str()
                )
            })
            .to_string();
    }
    result
}

fn obfuscate_code(input: &str) -> String {
    COUNTER.store(0, Ordering::SeqCst);

    let java_keywords: [&'static str; 54] = [
        "abstract",
        "assert",
        "boolean",
        "break",
        "byte",
        "case",
        "catch",
        "char",
        "class",
        "const",
        "continue",
        "default",
        "do",
        "double",
        "else",
        "enum",
        "extends",
        "final",
        "finally",
        "float",
        "for",
        "goto",
        "if",
        "implements",
        "import",
        "instanceof",
        "int",
        "interface",
        "long",
        "native",
        "new",
        "null",
        "package",
        "private",
        "protected",
        "public",
        "return",
        "short",
        "static",
        "strictfp",
        "super",
        "switch",
        "synchronized",
        "this",
        "throw",
        "throws",
        "transient",
        "try",
        "void",
        "volatile",
        "while",
        "true",
        "false",
        "IOException",
    ];

    let protected_input = find_protected_ranges(input);

    let re_sig = Regex::new(r"(?m)\bfunc_\d+\s*\(([^)]*)\)").unwrap();
    let re_strip_anno = Regex::new(r"@\w+(?:\([^)]*\))?\s*").unwrap();
    let re_param_ident = Regex::new(r"([A-Za-z_][A-Za-z0-9_]*)\s*(?:\[\s*\])*$").unwrap();

    let mut replacements: Vec<(String, String)> = Vec::new();

    for caps in re_sig.captures_iter(input) {
        let params = caps.get(1).unwrap().as_str();
        for raw in params.split(',') {
            let p = raw.trim();
            if p.is_empty() {
                continue;
            }

            let mut p2 = re_strip_anno.replace_all(p, "").into_owned();
            p2 = p2.replace("final ", "").replace("...", "");

            if let Some(id_caps) = re_param_ident.captures(&p2) {
                let ident = id_caps.get(1).unwrap().as_str();

                if !java_keywords.contains(&ident) && !replacements.iter().any(|(o, _)| o == ident)
                {
                    let id = COUNTER.fetch_add(1, Ordering::SeqCst) + 1;
                    replacements.push((ident.to_string(), format!("var_{}", id)));
                }
            }
        }
    }

    let re_vars = Regex::new(
    r"(?m)(?:^|\s)(?:(?:public|private|protected|static|final|volatile|transient)\s+)*([A-Za-z_][A-Za-z0-9_]*(?:<[^>]+>)?(?:\[\s*\])?)\s+([A-Za-z_][A-Za-z0-9_]*)(\s*(?:\[\s*\])?)(?:\s*[=;,){]|$)"
).unwrap();

    let result_after_decl_rename = re_vars
        .replace_all(input, |caps: &regex::Captures| {
            let start = caps.get(0).unwrap().start();
            if is_in_ranges(start, &protected_input) {
                return caps[0].to_string();
            }

            let ident = caps.get(2).unwrap().as_str();

            // Skip function names (start with 'func_'), class/interface declarations, and keywords
            if ident.starts_with("func_")
                || caps.get(1).unwrap().as_str() == "class"
                || caps.get(1).unwrap().as_str() == "interface"
                || java_keywords.contains(&ident)
            {
                return caps[0].to_string();
            }

            if !replacements.iter().any(|(o, _)| o == ident) {
                let id = COUNTER.fetch_add(1, Ordering::SeqCst) + 1;
                replacements.push((ident.to_string(), format!("var_{}", id)));
            }

            let new_name = &replacements.iter().find(|(o, _)| o == ident).unwrap().1;
            let array_suf = caps.get(3).map_or("", |m| m.as_str());

            // Preserve everything exactly as it was, only replacing the identifier
            let full_match = caps.get(0).unwrap().as_str();
            let before_ident =
                &full_match[..caps.get(2).unwrap().start() - caps.get(0).unwrap().start()];
            let after_ident =
                &full_match[caps.get(2).unwrap().end() - caps.get(0).unwrap().start()..];

            // Remove any extra space before equals while preserving other spacing
            let cleaned_after = if after_ident.starts_with(" =") {
                &after_ident[1..] // Remove one space
            } else {
                after_ident
            };

            format!("{}{}{}{}", before_ident, new_name, array_suf, cleaned_after)
        })
        .to_string();

    let protected_after = find_protected_ranges(&result_after_decl_rename);

    apply_replacements_vars(&result_after_decl_rename, &replacements, &protected_after)
}

pub fn obfuscate(input_file: &str, output_file: &str) -> io::Result<()> {
    let code = fs::read_to_string(input_file)?;
    let func_name_obfuscated = obfuscate_function_names(&code);
    let obfuscated = obfuscate_code(&func_name_obfuscated);
    fs::write(output_file, obfuscated)?;
    Ok(())
}

#[cfg(test)]
mod tests {

    #[test]
    fn test_obfuscate_function_names() {
        let input = "public class Test { public void myFunction() {} }";
        let expected = "public class Test { public void func_1() {} }";
        let result = super::obfuscate_function_names(input);
        println!("Result: {}", result);
        assert_eq!(result, expected);
    }

    #[test]
    fn test_obfuscate_code() {
        let input = "public class Test { @Test void isValidSignature_with_invalid_data_test() { int result = importantVar1 + importantVar2; return result; } }";
        let expected = "public class Test { @Test void func_1() { int var_1 = importantVar1 + importantVar2; return var_1; } }";
        let func_name_obfuscated = super::obfuscate_function_names(input);
        let result = super::obfuscate_code(&func_name_obfuscated);
        println!("Result: {}", result);
        assert_eq!(result, expected);
    }
}
