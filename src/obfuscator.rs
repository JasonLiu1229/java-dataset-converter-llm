use std::collections::HashMap;
use std::fs;
use std::io;

use tree_sitter::{Node, Parser, TreeCursor};

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

fn obfuscate_code(java_code: &str) -> String {
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

    let mut replacements: Vec<Replacement> = Vec::new();

    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.kind() == "method_declaration" {
            process_method(node, java_code, &mut replacements);
        }

        let mut c = node.walk();
        for child in node.children(&mut c) {
            stack.push(child);
        }
    }

    apply_replacements(java_code, &replacements)
}

fn process_method(method: Node, source: &str, replacements: &mut Vec<Replacement>) {
    let bytes = source.as_bytes();

    let mut var_counter: usize = 1;
    let mut var_map: HashMap<String, String> = HashMap::new();

    for i in 0..method.child_count() {
        if let Some(child) = method.child(i) {
            if child.kind() == "formal_parameters" {
                let mut c2 = child.walk();
                for param in child.children(&mut c2) {
                    if param.kind() == "formal_parameter" || param.kind() == "receiver_parameter" {
                        if let Some(name_node) = param.child_by_field_name("name") {
                            if let Ok(name) = name_node.utf8_text(bytes) {
                                let name = name.to_string();
                                if !var_map.contains_key(&name) {
                                    let new_name = format!("var_{}", var_counter);
                                    var_counter += 1;
                                    var_map.insert(name, new_name);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    let body_opt = method.child_by_field_name("body");

    if let Some(body) = body_opt {
        let mut stack = vec![body];
        while let Some(node) = stack.pop() {
            if node.kind() == "local_variable_declaration" {
                let mut c = node.walk();
                for ch in node.children(&mut c) {
                    if ch.kind() == "variable_declarator" {
                        if let Some(name_node) = ch.child_by_field_name("name") {
                            if let Ok(name) = name_node.utf8_text(bytes) {
                                let name = name.to_string();
                                if !var_map.contains_key(&name) {
                                    let new_name = format!("var_{}", var_counter);
                                    var_counter += 1;
                                    var_map.insert(name, new_name);
                                }
                            }
                        }
                    }
                }
            }

            let mut c = node.walk();
            for ch in node.children(&mut c) {
                stack.push(ch);
            }
        }

        let mut stack2 = vec![body];
        while let Some(node) = stack2.pop() {
            if node.kind() == "identifier" {
                if let Ok(name) = node.utf8_text(bytes) {
                    if let Some(new_name) = var_map.get(name) {
                        replacements.push(Replacement {
                            start: node.start_byte() as usize,
                            end: node.end_byte() as usize,
                            text: new_name.clone(),
                        });
                    }
                }
            }

            let mut c = node.walk();
            for ch in node.children(&mut c) {
                stack2.push(ch);
            }
        }
    }
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
