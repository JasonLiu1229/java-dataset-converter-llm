// Disclaimer: Code is made using GPT-5.2 and may require further review.

use std::collections::HashMap;
use std::fs;
use std::io;

use tree_sitter::{Node, Parser};

#[derive(Debug, Clone)]
struct Replacement {
    start: usize,
    end: usize,
    text: String,
}

fn apply_replacements(source: &str, replacements: &[Replacement]) -> String {
    let mut bytes = source.as_bytes().to_vec();

    // Sort back-to-front so indices remain valid.
    let mut reps: Vec<_> = replacements.to_vec();
    reps.sort_by_key(|r| std::cmp::Reverse(r.start));

    for r in reps {
        if r.start <= r.end && r.end <= bytes.len() {
            bytes.splice(r.start..r.end, r.text.as_bytes().iter().copied());
        }
    }

    match String::from_utf8(bytes) {
        Ok(s) => s,
        Err(e) => String::from_utf8_lossy(&e.into_bytes()).into_owned(),
    }
}

fn is_ident_byte(b: u8) -> bool {
    matches!(b, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'$')
}

/// Trim a (start,end) byte span down to exactly the identifier token.
fn trim_to_identifier_span(
    source: &str,
    mut start: usize,
    mut end: usize,
) -> Option<(usize, usize)> {
    let bytes = source.as_bytes();

    if start > end || end > bytes.len() {
        return None;
    }

    while start < end && !is_ident_byte(bytes[start]) {
        start += 1;
    }
    while end > start && !is_ident_byte(bytes[end - 1]) {
        end -= 1;
    }
    if start >= end {
        return None;
    }

    let mut e = start;
    while e < end && is_ident_byte(bytes[e]) {
        e += 1;
    }

    if e > start { Some((start, e)) } else { None }
}

fn same_span(a: Node, b: Node) -> bool {
    a.start_byte() == b.start_byte() && a.end_byte() == b.end_byte()
}

fn is_field_node(node: Node, parent: Node, field: &str) -> bool {
    parent
        .child_by_field_name(field)
        .map(|f| same_span(f, node))
        .unwrap_or(false)
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
    let mut replacements: Vec<Replacement> = Vec::new();
    let mut func_counter: usize = 1;

    fn walk(
        node: Node,
        source: &str,
        func_counter: &mut usize,
        replacements: &mut Vec<Replacement>,
    ) {
        if node.kind() == "method_declaration" {
            if let Some(name_node) = node.child_by_field_name("name") {
                let start0 = name_node.start_byte() as usize;
                let end0 = name_node.end_byte() as usize;

                if let Some((start, end)) = trim_to_identifier_span(source, start0, end0) {
                    let new_name = format!("func_{}", *func_counter);
                    *func_counter += 1;
                    replacements.push(Replacement {
                        start,
                        end,
                        text: new_name,
                    });
                }
            }
        }

        let mut cursor = node.walk();
        if cursor.goto_first_child() {
            loop {
                walk(cursor.node(), source, func_counter, replacements);
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
        }
    }

    walk(root, java_code, &mut func_counter, &mut replacements);
    apply_replacements(java_code, &replacements)
}

fn is_non_variable_identifier_context(ident: Node) -> bool {
    let Some(parent) = ident.parent() else {
        return false;
    };
    let pk = parent.kind();

    if pk == "method_invocation" && is_field_node(ident, parent, "name") {
        return true;
    }

    if pk == "field_access" && is_field_node(ident, parent, "field") {
        return true;
    }

    if pk == "method_reference" && is_field_node(ident, parent, "name") {
        return true;
    }

    if (pk == "labeled_statement" && is_field_node(ident, parent, "label"))
        || (pk == "break_statement" && is_field_node(ident, parent, "label"))
        || (pk == "continue_statement" && is_field_node(ident, parent, "label"))
    {
        return true;
    }

    if (pk == "variable_declarator" && is_field_node(ident, parent, "name"))
        || (pk == "formal_parameter" && is_field_node(ident, parent, "name"))
        || (pk == "catch_formal_parameter" && is_field_node(ident, parent, "name"))
        || (pk == "resource" && is_field_node(ident, parent, "name"))
    {
        return true;
    }

    false
}

fn lookup_scope(scopes: &[HashMap<String, String>], name: &str) -> Option<String> {
    for scope in scopes.iter().rev() {
        if let Some(v) = scope.get(name) {
            return Some(v.clone());
        }
    }
    None
}

fn declare_identifier(
    name_node: Node,
    java_code: &str,
    scopes: &mut [HashMap<String, String>],
    replacements: &mut Vec<Replacement>,
    local_var_counter: &mut usize,
) {
    let start0 = name_node.start_byte() as usize;
    let end0 = name_node.end_byte() as usize;
    let Some((start, end)) = trim_to_identifier_span(java_code, start0, end0) else {
        return;
    };

    let name = &java_code[start..end];
    let new_name = format!("var_{}", *local_var_counter);
    *local_var_counter += 1;

    if let Some(last) = scopes.last_mut() {
        last.insert(name.to_string(), new_name.clone());
    }

    replacements.push(Replacement {
        start,
        end,
        text: new_name,
    });
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
    let mut local_var_counter: usize = 1;

    fn obfuscate_method(
        method: Node,
        java_code: &str,
        replacements: &mut Vec<Replacement>,
        local_var_counter: &mut usize,
    ) {
        // Scope stack: method scope at bottom.
        let mut scopes: Vec<HashMap<String, String>> = vec![HashMap::new()];

        // Parameters are in method scope.
        if let Some(params) = method.child_by_field_name("parameters") {
            let mut c = params.walk();
            if c.goto_first_child() {
                loop {
                    let p = c.node();
                    if p.kind() == "formal_parameter" {
                        if let Some(name_node) = p.child_by_field_name("name") {
                            declare_identifier(
                                name_node,
                                java_code,
                                &mut scopes,
                                replacements,
                                local_var_counter,
                            );
                        }
                    }
                    if !c.goto_next_sibling() {
                        break;
                    }
                }
            }
        }

        let Some(body) = method.child_by_field_name("body") else {
            return;
        };

        fn walk(
            node: Node,
            java_code: &str,
            scopes: &mut Vec<HashMap<String, String>>,
            replacements: &mut Vec<Replacement>,
            local_var_counter: &mut usize,
        ) {
            // Enter new scope for blocks
            let opens_block_scope = node.kind() == "block";

            // Enter new scope for lambdas (they introduce their own params)
            let opens_lambda_scope = node.kind() == "lambda_expression";

            if opens_block_scope || opens_lambda_scope {
                scopes.push(HashMap::new());
            }

            // Handle lambda parameters (in lambda scope)
            if node.kind() == "lambda_expression" {
                if let Some(params) = node.child_by_field_name("parameters") {
                    let mut c = params.walk();
                    if c.goto_first_child() {
                        loop {
                            let p = c.node();
                            // Tree-sitter-java may represent lambda params as:
                            // - identifier (single param)
                            // - formal_parameter
                            if p.kind() == "formal_parameter" {
                                if let Some(name_node) = p.child_by_field_name("name") {
                                    declare_identifier(
                                        name_node,
                                        java_code,
                                        scopes,
                                        replacements,
                                        local_var_counter,
                                    );
                                }
                            } else if p.kind() == "identifier" {
                                // single-identifier param
                                declare_identifier(
                                    p,
                                    java_code,
                                    scopes,
                                    replacements,
                                    local_var_counter,
                                );
                            }
                            if !c.goto_next_sibling() {
                                break;
                            }
                        }
                    }
                }
            }

            // Local variable declarations: int x = 0;  (also supports: int a=1, b=2;)
            if node.kind() == "local_variable_declaration" {
                let mut c = node.walk();
                if c.goto_first_child() {
                    loop {
                        let child = c.node();
                        if child.kind() == "variable_declarator" {
                            if let Some(name_node) = child.child_by_field_name("name") {
                                declare_identifier(
                                    name_node,
                                    java_code,
                                    scopes,
                                    replacements,
                                    local_var_counter,
                                );
                            }
                        }
                        if !c.goto_next_sibling() {
                            break;
                        }
                    }
                }
            }

            // Enhanced for: for (Type x : expr)
            if node.kind() == "enhanced_for_statement" {
                if let Some(var) = node.child_by_field_name("variable") {
                    // Often a formal_parameter or local_variable_declaration-ish structure
                    if var.kind() == "formal_parameter" {
                        if let Some(name_node) = var.child_by_field_name("name") {
                            declare_identifier(
                                name_node,
                                java_code,
                                scopes,
                                replacements,
                                local_var_counter,
                            );
                        }
                    } else if var.kind() == "local_variable_declaration" {
                        let mut c = var.walk();
                        if c.goto_first_child() {
                            loop {
                                let ch = c.node();
                                if ch.kind() == "variable_declarator" {
                                    if let Some(name_node) = ch.child_by_field_name("name") {
                                        declare_identifier(
                                            name_node,
                                            java_code,
                                            scopes,
                                            replacements,
                                            local_var_counter,
                                        );
                                    }
                                }
                                if !c.goto_next_sibling() {
                                    break;
                                }
                            }
                        }
                    } else if var.kind() == "identifier" {
                        declare_identifier(var, java_code, scopes, replacements, local_var_counter);
                    }
                }
            }

            // Catch clause parameter: catch (Exception e)
            if node.kind() == "catch_clause" {
                if let Some(param) = node.child_by_field_name("parameter") {
                    if param.kind() == "catch_formal_parameter"
                        || param.kind() == "formal_parameter"
                    {
                        if let Some(name_node) = param.child_by_field_name("name") {
                            declare_identifier(
                                name_node,
                                java_code,
                                scopes,
                                replacements,
                                local_var_counter,
                            );
                        }
                    }
                }
            }

            // Try-with-resources: try (InputStream in = ...)
            if node.kind() == "resource" {
                if let Some(name_node) = node.child_by_field_name("name") {
                    declare_identifier(
                        name_node,
                        java_code,
                        scopes,
                        replacements,
                        local_var_counter,
                    );
                }
            }

            // Identifier usages (variable references)
            if node.kind() == "identifier" {
                if !is_non_variable_identifier_context(node) {
                    let start0 = node.start_byte() as usize;
                    let end0 = node.end_byte() as usize;

                    if let Some((start, end)) = trim_to_identifier_span(java_code, start0, end0) {
                        let name = &java_code[start..end];

                        // Don't rename types
                        let parent_kind = node.parent().map(|p| p.kind()).unwrap_or("");
                        if parent_kind != "type_identifier"
                            && parent_kind != "scoped_type_identifier"
                        {
                            if let Some(new_name) = lookup_scope(scopes, name) {
                                replacements.push(Replacement {
                                    start,
                                    end,
                                    text: new_name,
                                });
                            }
                        }
                    }
                }
            }

            // Walk children
            let mut cursor = node.walk();
            if cursor.goto_first_child() {
                loop {
                    walk(
                        cursor.node(),
                        java_code,
                        scopes,
                        replacements,
                        local_var_counter,
                    );
                    if !cursor.goto_next_sibling() {
                        break;
                    }
                }
            }

            if opens_block_scope || opens_lambda_scope {
                scopes.pop();
            }
        }

        walk(
            body,
            java_code,
            &mut scopes,
            replacements,
            local_var_counter,
        );
    }

    fn walk_methods(
        node: Node,
        java_code: &str,
        replacements: &mut Vec<Replacement>,
        local_var_counter: &mut usize,
    ) {
        if node.kind() == "method_declaration" {
            obfuscate_method(node, java_code, replacements, local_var_counter);
        }

        let mut cursor = node.walk();
        if cursor.goto_first_child() {
            loop {
                walk_methods(cursor.node(), java_code, replacements, local_var_counter);
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
        }
    }

    walk_methods(root, java_code, &mut replacements, &mut local_var_counter);

    let mut dedup: HashMap<(usize, usize), String> = HashMap::new();
    for r in replacements {
        dedup.insert((r.start, r.end), r.text);
    }
    let mut replacements: Vec<Replacement> = dedup
        .into_iter()
        .map(|((s, e), t)| Replacement {
            start: s,
            end: e,
            text: t,
        })
        .collect();

    replacements.sort_by_key(|r| (r.start, r.end));

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
        assert_eq!(result, expected);
    }

    #[test]
    fn test_obfuscate_code() {
        let input = "public class Test { public void myFunction(int param1) { int x = 0; x = x + param1; } }";
        let func_name_obfuscated = super::obfuscate_function_names(input);
        let expected = "public class Test { public void func_1(int var_1) { int var_2 = 0; var_2 = var_2 + var_1; } }";
        let result = super::obfuscate_code(&func_name_obfuscated);
        assert_eq!(result, expected);
    }

    #[test]
    fn test_preserves_equals() {
        let input =
            r#"public class T { public void m() { Listener listener = new Listener("table"); } }"#;
        let func_name_obfuscated = super::obfuscate_function_names(input);
        let result = super::obfuscate_code(&func_name_obfuscated);

        assert!(result.contains("= new Listener(\"table\")"));
    }

    #[test]
    fn test_does_not_rename_method_name() {
        let input = r#"public class T { public void m() { int size = 1; foo.size(); } }"#;
        let func_name_obfuscated = super::obfuscate_function_names(input);
        let result = super::obfuscate_code(&func_name_obfuscated);

        assert!(result.contains(".size()"));
    }

    #[test]
    fn test_shadowing_scopes() {
        let input = r#"public class T { public void m() { int x = 1; { int x = 2; x = x + 1; } x = x + 1; } }"#;
        let func_name_obfuscated = super::obfuscate_function_names(input);
        let result = super::obfuscate_code(&func_name_obfuscated);

        assert!(!result.contains(" int x "));
    }

    #[test]
    fn test_enhanced_for_and_catch() {
        let input = r#"
            public class T {
                public void m(java.util.List<String> list) {
                    for (String s : list) { System.out.println(s); }
                    try { throw new RuntimeException(); }
                    catch (Exception e) { System.out.println(e); }
                }
            }
        "#;
        let func_name_obfuscated = super::obfuscate_function_names(input);
        let result = super::obfuscate_code(&func_name_obfuscated);

        assert!(result.contains("for (String"));
        assert!(result.contains("catch (Exception"));
    }
}
