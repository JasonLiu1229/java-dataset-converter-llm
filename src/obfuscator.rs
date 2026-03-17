// Disclaimer: Code is made using help of AI, so errors or some things might not be perfect.

use std::collections::HashMap;
use std::fs;
use std::io;

use tree_sitter::{Node, Parser};

use crate::sanitizer::sanitize_structural;

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
        || (pk == "resource" && is_field_node(ident, parent, "name"))
    {
        return true;
    }

    // catch_formal_parameter has no "name" field — the variable is just the last
    // plain identifier child. Mark ALL identifiers inside it as declaration sites
    // so the generic usage-lookup does not double-replace the declaration byte range.
    if pk == "catch_formal_parameter" {
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

fn collect_class_fields(
    node: Node,
    java_code: &str,
    class_scope: &mut HashMap<String, String>,
    counter: &mut usize,
) {
    if node.kind() == "field_declaration" {
        let mut c = node.walk();
        if c.goto_first_child() {
            loop {
                let ch = c.node();
                if ch.kind() == "variable_declarator" {
                    if let Some(name_node) = ch.child_by_field_name("name") {
                        let start = name_node.start_byte();
                        let end = name_node.end_byte();
                        if let Some((s, e)) = trim_to_identifier_span(java_code, start, end) {
                            let name = &java_code[s..e];
                            let new_name = format!("var_{}", *counter);
                            *counter += 1;
                            class_scope.insert(name.to_string(), new_name);
                        }
                    }
                }
                if !c.goto_next_sibling() {
                    break;
                }
            }
        }
        // Don't recurse into field_declaration children further.
        return;
    }

    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            collect_class_fields(cursor.node(), java_code, class_scope, counter);
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
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

    let mut class_scope: HashMap<String, String> = HashMap::new();
    collect_class_fields(root, java_code, &mut class_scope, &mut local_var_counter);

    // Also emit replacements for the field declaration sites themselves.
    // (The field names at declaration must be renamed in the output too.)
    fn emit_field_declaration_replacements(
        node: Node,
        java_code: &str,
        class_scope: &HashMap<String, String>,
        replacements: &mut Vec<Replacement>,
    ) {
        if node.kind() == "field_declaration" {
            let mut c = node.walk();
            if c.goto_first_child() {
                loop {
                    let ch = c.node();
                    if ch.kind() == "variable_declarator" {
                        if let Some(name_node) = ch.child_by_field_name("name") {
                            let start = name_node.start_byte();
                            let end = name_node.end_byte();
                            if let Some((s, e)) = trim_to_identifier_span(java_code, start, end) {
                                let name = &java_code[s..e];
                                if let Some(new_name) = class_scope.get(name) {
                                    replacements.push(Replacement {
                                        start: s,
                                        end: e,
                                        text: new_name.clone(),
                                    });
                                }
                            }
                        }
                    }
                    if !c.goto_next_sibling() {
                        break;
                    }
                }
            }
            return;
        }
        let mut cursor = node.walk();
        if cursor.goto_first_child() {
            loop {
                emit_field_declaration_replacements(
                    cursor.node(),
                    java_code,
                    class_scope,
                    replacements,
                );
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
        }
    }

    emit_field_declaration_replacements(root, java_code, &class_scope, &mut replacements);

    fn obfuscate_method(
        method: Node,
        java_code: &str,
        replacements: &mut Vec<Replacement>,
        local_var_counter: &mut usize,
        class_scope: HashMap<String, String>,
    ) {
        let mut scopes: Vec<HashMap<String, String>> = vec![class_scope];

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

            let opens_for_scope = matches!(node.kind(), "for_statement" | "enhanced_for_statement");

            if opens_block_scope || opens_lambda_scope || opens_for_scope {
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
            // tree-sitter-java exposes the loop variable via the "name" field on
            // enhanced_for_statement (added in recent grammar versions).  Fall back
            // to scanning for the last identifier before ":" for older grammars.
            if node.kind() == "enhanced_for_statement" {
                // Prefer the dedicated "name" field if the grammar exposes it.
                let name_node_opt = node.child_by_field_name("name");

                if let Some(name_node) = name_node_opt {
                    declare_identifier(
                        name_node,
                        java_code,
                        scopes,
                        replacements,
                        local_var_counter,
                    );
                } else {
                    // Fallback: Walk children and collect identifiers until we hit ":"
                    let mut c = node.walk();
                    let mut last_ident: Option<Node> = None;
                    if c.goto_first_child() {
                        loop {
                            let ch = c.node();
                            if ch.kind() == ":" {
                                break; // everything after this is the iterable, not the var
                            }
                            if ch.kind() == "identifier" {
                                last_ident = Some(ch);
                            }
                            if !c.goto_next_sibling() {
                                break;
                            }
                        }
                    }
                    if let Some(name_node) = last_ident {
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

            // Catch clause parameter: catch (Exception e)
            if node.kind() == "catch_clause" {
                if let Some(param) = node.child_by_field_name("parameter") {
                    if param.kind() == "catch_formal_parameter"
                        || param.kind() == "formal_parameter"
                    {
                        let mut name_node: Option<Node> = None;
                        let mut c = param.walk();
                        if c.goto_first_child() {
                            loop {
                                if c.node().kind() == "identifier" {
                                    name_node = Some(c.node());
                                }
                                if !c.goto_next_sibling() {
                                    break;
                                }
                            }
                        }
                        if let Some(n) = name_node {
                            declare_identifier(
                                n,
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
            // node.kind() == "identifier" means it is a plain identifier token.
            // tree-sitter-java uses "type_identifier" for type names, so plain
            // "identifier" nodes are always variable/constant references — never
            // raw type names.  We still guard against a few annotation / label
            // contexts that are caught by is_non_variable_identifier_context.
            if node.kind() == "identifier" {
                if !is_non_variable_identifier_context(node) {
                    let start0 = node.start_byte() as usize;
                    let end0 = node.end_byte() as usize;

                    if let Some((start, end)) = trim_to_identifier_span(java_code, start0, end0) {
                        let name = &java_code[start..end];

                        let skip_type = node
                            .parent()
                            .map(|p| {
                                matches!(
                                    p.kind(),
                                    "type_identifier"
                                        | "scoped_type_identifier"
                                        | "generic_type"
                                        | "array_type"
                                )
                            })
                            .unwrap_or(false);

                        if !skip_type {
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

            if opens_block_scope || opens_lambda_scope || opens_for_scope {
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
        class_scope: &HashMap<String, String>,
    ) {
        // Do not descend into ERROR nodes.  When tree-sitter cannot parse a
        // region it wraps it in an ERROR node whose children are flattened
        // tokens — method_declaration nodes inside it are gone, so we would
        // produce no renames anyway.  Skipping eagerly avoids emitting
        // spurious replacements for tokens that only superficially look like
        // identifiers.
        if node.is_error() {
            return;
        }

        if node.kind() == "method_declaration" {
            obfuscate_method(
                node,
                java_code,
                replacements,
                local_var_counter,
                class_scope.clone(),
            );
        }

        let mut cursor = node.walk();
        if cursor.goto_first_child() {
            loop {
                walk_methods(
                    cursor.node(),
                    java_code,
                    replacements,
                    local_var_counter,
                    class_scope,
                );
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
        }
    }

    walk_methods(
        root,
        java_code,
        &mut replacements,
        &mut local_var_counter,
        &class_scope,
    );

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

/// Obfuscate Java source that has already been through [`sanitize_structural`],
/// returning the result as a `String` without touching the filesystem.
///
/// This is the primary entry point used by `main` so that no intermediate
/// obfuscated `.java` files need to be written to disk.
pub fn obfuscate_str(sanitized_src: &str) -> io::Result<String> {
    let func_name_obfuscated = obfuscate_function_names(sanitized_src);
    Ok(obfuscate_code(&func_name_obfuscated))
}

/// File-based wrapper kept for CLI tooling that wants obfuscated `.java` files
/// on disk (e.g. for inspection or partial re-runs).
pub fn obfuscate(input_file: &str, output_file: &str) -> io::Result<()> {
    let raw_code = fs::read_to_string(input_file)?;
    // Use sanitize_structural only — no backslash collapsing — so the file on
    // disk is in the same state that generate_jsonl expects to read back.
    let sanitized = sanitize_structural(&raw_code);
    let result = obfuscate_str(&sanitized)?;
    fs::write(output_file, result)?;
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

    #[test]
    fn test_class_fields_obfuscated() {
        let input = r#"
            public class T {
                private int counter;
                private String label;
                public void m() {
                    counter = counter + 1;
                    label = "hello";
                }
            }
        "#;
        let func_name_obfuscated = super::obfuscate_function_names(input);
        let result = super::obfuscate_code(&func_name_obfuscated);

        assert!(
            !result.contains("counter"),
            "field 'counter' should be renamed"
        );
        assert!(!result.contains("label"), "field 'label' should be renamed");
    }

    #[test]
    fn test_class_fields_and_for_loop_vars_renamed() {
        let input = r#"
        public class TestClass77051 {
            private Carte carte;
            private Noeud n1, n3, n5, n7;
 
            @Test public void testFiltreNoeudsSimples() {
                carte.filtreNoeudsSimples();
                for (Arc a : carte.getPopArcs()) {
                    if (a.getNoeudIni() == n7) {
                        Assert.assertEquals(n1, a.getNoeudFin());
                    }
                }
                for (Noeud n : carte.getPopNoeuds()) {
                    System.out.println(n);
                }
            }
        }
    "#;
        let step1 = super::obfuscate_function_names(input);
        let result = super::obfuscate_code(&step1);

        // Method name renamed
        assert!(
            !result.contains("testFiltreNoeudsSimples"),
            "method name should be renamed"
        );
        // Class fields renamed everywhere
        assert!(!result.contains("carte"), "field 'carte' should be renamed");
        assert!(!result.contains(" n1"), "field 'n1' should be renamed");
        assert!(!result.contains(" n7"), "field 'n7' should be renamed");
        // For-loop variables renamed
        assert!(
            !result.contains(" a ") && !result.contains("(Arc a"),
            "loop var 'a' should be renamed"
        );
        assert!(
            !result.contains(" n ") && !result.contains("(Noeud n"),
            "loop var 'n' should be renamed"
        );
        // Types must NOT be renamed
        assert!(result.contains("Arc"), "type 'Arc' must be preserved");
        assert!(result.contains("Noeud"), "type 'Noeud' must be preserved");
        // Method calls on objects must NOT be renamed
        assert!(
            result.contains(".filtreNoeudsSimples()"),
            "invoked method name must be preserved"
        );
        assert!(
            result.contains(".getPopArcs()"),
            "invoked method name must be preserved"
        );
        assert!(
            result.contains(".getNoeudIni()"),
            "invoked method name must be preserved"
        );
    }

    #[test]
    fn test_nested_enhanced_for_loops() {
        let input = r#"
        public class T {
            public void m(java.util.List<java.util.List<String>> matrix) {
                for (java.util.List<String> row : matrix) {
                    for (String cell : row) {
                        System.out.println(cell);
                    }
                }
            }
        }
    "#;
        let step1 = super::obfuscate_function_names(input);
        let result = super::obfuscate_code(&step1);

        assert!(
            !result.contains(" row"),
            "outer loop var 'row' should be renamed"
        );
        assert!(
            !result.contains(" cell"),
            "inner loop var 'cell' should be renamed"
        );
        // The two vars should get different names
        let var_names: Vec<&str> = result
            .split_whitespace()
            .filter(|w| w.starts_with("var_"))
            .collect();
        let unique: std::collections::HashSet<_> = var_names.iter().collect();
        assert!(
            unique.len() >= 2,
            "outer and inner loop vars should get distinct names"
        );
    }

    #[test]
    fn test_classic_for_loop_variable() {
        let input = r#"
        public class T {
            public void m() {
                for (int i = 0; i < 10; i++) {
                    System.out.println(i);
                }
            }
        }
    "#;
        let step1 = super::obfuscate_function_names(input);
        let result = super::obfuscate_code(&step1);

        assert!(
            !result.contains(" i ") && !result.contains("(int i"),
            "for-init variable 'i' should be renamed"
        );
    }

    #[test]
    fn test_multiple_methods_independent_scopes() {
        let input = r#"
        public class T {
            public void first() {
                int x = 1;
                System.out.println(x);
            }
            public void second() {
                int x = 2;
                System.out.println(x);
            }
        }
    "#;
        let step1 = super::obfuscate_function_names(input);
        let result = super::obfuscate_code(&step1);

        // Both `x` variables must be gone
        assert!(
            !result.contains(" x "),
            "local 'x' in both methods should be renamed"
        );
        // Method names renamed
        assert!(
            !result.contains("first"),
            "method 'first' should be renamed"
        );
        assert!(
            !result.contains("second"),
            "method 'second' should be renamed"
        );
    }

    #[test]
    fn test_local_shadows_class_field() {
        let input = r#"
        public class T {
            private int value;
            public void m() {
                int value = 42;       // shadows field
                System.out.println(value);
            }
        }
    "#;
        let step1 = super::obfuscate_function_names(input);
        let result = super::obfuscate_code(&step1);

        assert!(
            !result.contains(" value"),
            "both field and local 'value' should be renamed"
        );
    }

    #[test]
    fn test_try_with_resources_variable_renamed() {
        let input = r#"
        public class T {
            public void m() throws Exception {
                try (java.io.InputStream stream = new java.io.FileInputStream("f")) {
                    int b = stream.read();
                    System.out.println(b);
                }
            }
        }
    "#;
        let step1 = super::obfuscate_function_names(input);
        let result = super::obfuscate_code(&step1);

        assert!(
            !result.contains(" stream"),
            "resource var 'stream' should be renamed"
        );
        assert!(!result.contains(" b "), "local 'b' should be renamed");
        assert!(
            result.contains(".read()"),
            "method call '.read()' must be preserved"
        );
    }

    #[test]
    fn test_lambda_parameter_renamed() {
        let input = r#"
        public class T {
            public void m(java.util.List<String> items) {
                items.forEach(item -> System.out.println(item));
            }
        }
    "#;
        let step1 = super::obfuscate_function_names(input);
        let result = super::obfuscate_code(&step1);

        assert!(
            !result.contains(" item"),
            "lambda param 'item' should be renamed"
        );
        assert!(
            result.contains(".forEach("),
            "method call '.forEach' must be preserved"
        );
    }

    #[test]
    fn test_string_literals_untouched() {
        let input = r#"
        public class T {
            public void m() {
                int value = 1;
                String s = "value is not a variable here";
                System.out.println(s);
            }
        }
    "#;
        let step1 = super::obfuscate_function_names(input);
        let result = super::obfuscate_code(&step1);

        assert!(
            result.contains("\"value is not a variable here\""),
            "string literal content must be preserved verbatim"
        );
    }

    #[test]
    fn test_this_field_access_renamed() {
        let input = r#"
        public class T {
            private int counter;
            public void m() {
                this.counter = this.counter + 1;
            }
        }
    "#;
        let step1 = super::obfuscate_function_names(input);
        let result = super::obfuscate_code(&step1);

        assert!(
            !result.contains(" counter ="),
            "field declaration site 'counter' should be renamed"
        );
    }

    #[test]
    fn test_multiple_classes_independent() {
        let input = r#"
        public class A {
            private int x;
            public void ma() { x = 1; }
        }
        class B {
            private int x;
            public void mb() { x = 2; }
        }
    "#;
        let step1 = super::obfuscate_function_names(input);
        let result = super::obfuscate_code(&step1);

        assert!(
            !result.contains(" x "),
            "field 'x' in both classes should be renamed"
        );
        assert!(!result.contains("ma("), "method 'ma' should be renamed");
        assert!(!result.contains("mb("), "method 'mb' should be renamed");
    }

    /// Regression test for the failing dataset example with:
    ///   - final local vars
    ///   - array-type locals (byte[])
    ///   - scoped-type locals (BlockHash.Match)
    ///   - variables used as method-invocation objects (best_match.source_offset())
    ///   - re-assignment (time = ... - time)
    ///   - external identifiers that must NOT be renamed (hashed_all_Qs, huge_bh)
    #[test]
    fn test_search_string_finds_too_many_matches() {
        let input = r#"
        public class TestClass109508 {
            @Test public void SearchStringFindsTooManyMatches() {
                final int kTestSize = 1 << 20;
                byte[] huge_dictionary = new byte[kTestSize];
                long time = System.nanoTime();
                BlockHash.Match best_match = new BlockHash.Match();
                huge_bh.FindBestMatch(hashed_all_Qs, huge_target, kTestSize, 0, best_match);
                time = System.nanoTime() - time;
                double elapsed_time_in_us = time / 1000.0;
                Assert.assertTrue(best_match.source_offset() > 0);
                Assert.assertTrue(1000000 > elapsed_time_in_us);
            }
        }
    "#;

        let step1 = super::obfuscate_function_names(input);
        let result = super::obfuscate_code(&step1);

        // Method name must be renamed
        assert!(
            !result.contains("SearchStringFindsTooManyMatches"),
            "method name should be renamed"
        );

        // All declared local variables must be renamed
        assert!(
            !result.contains("kTestSize"),
            "final local 'kTestSize' should be renamed"
        );
        assert!(
            !result.contains("huge_dictionary"),
            "array local 'huge_dictionary' should be renamed"
        );
        assert!(
            !result.contains("best_match"),
            "scoped-type local 'best_match' should be renamed"
        );
        assert!(
            !result.contains("elapsed_time_in_us"),
            "local 'elapsed_time_in_us' should be renamed"
        );
        // 'time' appears in identifiers and string literals - check the declaration form
        assert!(
            !result.contains("long time"),
            "local 'time' declaration should be renamed"
        );

        // External references (undeclared in this class) must be left alone
        assert!(
            result.contains("hashed_all_Qs"),
            "external ref 'hashed_all_Qs' must not be renamed"
        );
        assert!(
            result.contains("huge_target"),
            "external ref 'huge_target' must not be renamed"
        );
        assert!(
            result.contains("huge_bh"),
            "external ref 'huge_bh' must not be renamed"
        );

        // Types and invoked method names must be preserved
        assert!(
            result.contains("BlockHash"),
            "type 'BlockHash' must be preserved"
        );
        assert!(
            result.contains(".source_offset()"),
            "method call '.source_offset()' must be preserved"
        );
        assert!(
            result.contains(".FindBestMatch("),
            "method call '.FindBestMatch' must be preserved"
        );
        assert!(
            result.contains("System.nanoTime()"),
            "static call must be preserved"
        );
    }
}
