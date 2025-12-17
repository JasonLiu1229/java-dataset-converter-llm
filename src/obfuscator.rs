// Disclaimer: Code is made using GPT-5.1 and may require further review. (too little time to refactor it myself)

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
    let mut bytes = source.as_bytes().to_vec();

    let mut reps: Vec<_> = replacements.to_vec();
    reps.sort_by_key(|r| std::cmp::Reverse(r.start));

    for r in reps {
        // Be defensive: never panic on bad spans; just skip them.
        if r.start <= r.end && r.end <= bytes.len() {
            bytes.splice(r.start..r.end, r.text.as_bytes().iter().copied());
        }
    }

    match String::from_utf8(bytes) {
        Ok(s) => s,
        Err(e) => String::from_utf8_lossy(e.as_bytes()).into_owned(),
    }
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

        if cursor.goto_first_child() {
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

    walk(root, &mut cursor, &mut func_counter, &mut replacements, java_code);

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
    let mut cursor = root.walk();

    let mut replacements: Vec<Replacement> = Vec::new();

    let mut local_var_counter: usize = 1;

    fn obfuscate_method(
        method: Node,
        _cursor: &mut TreeCursor,
        replacements: &mut Vec<Replacement>,
        java_code: &str,
        local_var_counter: &mut usize,
    ) {
        let mut var_map: HashMap<String, String> = HashMap::new();

        let params_node = method.child_by_field_name("parameters");

        if let Some(params) = params_node {
            let mut c = params.walk();
            if c.goto_first_child() {
                loop {
                    let p = c.node();
                    if p.kind() == "formal_parameter" {
                        if let Some(name_node) = p.child_by_field_name("name") {
                            let start = name_node.start_byte() as usize;
                            let end = name_node.end_byte() as usize;
                            let name = &java_code[start..end];
                            let new_name = format!("var_{}", *local_var_counter);
                            *local_var_counter += 1;

                            var_map.insert(name.to_string(), new_name.clone());
                            replacements.push(Replacement {
                                start,
                                end,
                                text: new_name,
                            });
                        }
                    }
                    if !c.goto_next_sibling() {
                        break;
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
                    if c.goto_first_child() {
                        loop {
                            let child = c.node();
                            if child.kind() == "variable_declarator" {
                                if let Some(name_node) = child.child_by_field_name("name") {
                                    let start = name_node.start_byte() as usize;
                                    let end = name_node.end_byte() as usize;
                                    let name = &java_code[start..end];
                                    let new_name = format!("var_{}", *local_var_counter);
                                    *local_var_counter += 1;

                                    var_map.insert(name.to_string(), new_name.clone());
                                    replacements.push(Replacement {
                                        start,
                                        end,
                                        text: new_name,
                                    });
                                }
                            }

                            if !c.goto_next_sibling() {
                                break;
                            }
                        }
                    }
                }

                let mut c = node.walk();
                if c.goto_first_child() {
                    loop {
                        stack.push(c.node());
                        if !c.goto_next_sibling() {
                            break;
                        }
                    }
                }
            }
        }

        let body_opt = method.child_by_field_name("body");

        if let Some(body) = body_opt {
            let mut stack = vec![body];
            while let Some(node) = stack.pop() {
                if node.kind() == "identifier" {
                    let start = node.start_byte() as usize;
                    let end = node.end_byte() as usize;
                    let name = &java_code[start..end];

                    if let Some(new_name) = var_map.get(name) {
                        replacements.push(Replacement {
                            start,
                            end,
                            text: new_name.clone(),
                        });
                    }
                }

                let mut c = node.walk();
                if c.goto_first_child() {
                    loop {
                        stack.push(c.node());
                        if !c.goto_next_sibling() {
                            break;
                        }
                    }
                }
            }
        }
    }

    fn walk_methods(
        node: Node,
        cursor: &mut TreeCursor,
        replacements: &mut Vec<Replacement>,
        java_code: &str,
        local_var_counter: &mut usize,
    ) {
        if node.kind() == "method_declaration" {
            obfuscate_method(node, cursor, replacements, java_code, local_var_counter);
        }

        if cursor.goto_first_child() {
            loop {
                let child = cursor.node();
                walk_methods(child, cursor, replacements, java_code, local_var_counter);
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
            cursor.goto_parent();
        }
    }

    walk_methods(
        root,
        &mut cursor,
        &mut replacements,
        java_code,
        &mut local_var_counter,
    );

    apply_replacements(java_code, &replacements)
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
        let input = "public class Test { public void myFunction(int param1) { int x = 0; x = x + param1; } }";
        let func_name_obfuscated = super::obfuscate_function_names(input);
        let expected = "public class Test { public void func_1(int var_1) { int var_2 = 0; var_2 = var_2 + var_1; } }";
        let result = super::obfuscate_code(&func_name_obfuscated);
        println!("Result: {}", result);
        assert_eq!(result, expected);
    }
}
