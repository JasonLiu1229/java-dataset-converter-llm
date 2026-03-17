use crate::sanitizer::{fix_string_literals, sanitize_backslashes, sanitize_structural};
use serde::Serialize;
use std::fs;
use std::fs::File;
use std::io::{BufWriter, Write};

#[derive(Serialize)]
struct PromptResponse {
    prompt: String,
    response: String,
}

/// Write a JSONL training pair directly from in-memory source strings.
pub fn generate_jsonl_from_strings(
    original_src: &str,
    obfuscated_src: &str,
    output_file: &str,
) -> std::io::Result<()> {
    if !output_file.ends_with(".jsonl") {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Output file must have a .jsonl extension",
        ));
    }

    // ── Pipeline (phases 2 & 3) ──────────────────────────────────────────────
    // Phase 1 (sanitize_structural) was already applied by the caller.
    //
    // Phase 2: fix_string_literals — copies the prompt's literal contents into
    //   the response wherever they differ.  Obfuscation only renames identifiers,
    //   so any content difference in a literal is dataset corruption.  Must run
    //   BEFORE sanitize_backslashes because step 3 collapses backslash runs
    //   context-free; corrupt runs get collapsed differently from clean ones.
    //
    // Phase 3: sanitize_backslashes — both sides now have identical literal
    //   contents, so this step applies identically to both and token counts
    //   stay in sync.
    let original_src = fix_string_literals(obfuscated_src, original_src).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "String literal count mismatch between prompt and response — \
                 cannot repair corrupt pair",
        )
    })?;

    let obfuscated_code = sanitize_backslashes(obfuscated_src);
    let original_code = sanitize_backslashes(&original_src);

    if obfuscated_code.trim().is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Obfuscated source is empty or whitespace-only — \
             the obfuscation step may not have run yet",
        ));
    }
    if original_code.trim().is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Original source is empty or whitespace-only",
        ));
    }

    let mut writer = BufWriter::new(File::create(output_file)?);
    let pair = PromptResponse {
        prompt: obfuscated_code,
        response: original_code,
    };
    let json_line = serde_json::to_string(&pair)?;
    writeln!(writer, "{}", json_line)?;

    Ok(())
}

/// File-based wrapper kept for use in tests and any CLI tooling that already
/// has both files on disk.  Applies the full three-phase sanitisation pipeline
/// internally (reads files → sanitize_structural → fix_string_literals →
/// sanitize_backslashes → write JSONL).
pub fn generate_jsonl(
    original_file: &str,
    obfuscated_file: &str,
    output_file: &str,
) -> std::io::Result<()> {
    let original_p1 = sanitize_structural(&fs::read_to_string(original_file)?);
    let obfuscated_p1 = sanitize_structural(&fs::read_to_string(obfuscated_file)?);
    generate_jsonl_from_strings(&original_p1, &obfuscated_p1, output_file)
}
#[cfg(test)]
mod tests {
    use super::generate_jsonl_from_strings;
    use regex::Regex;
    use std::fs;
    use std::io::ErrorKind;
    use tempfile::NamedTempFile;

    fn tokenize(code: &str) -> Vec<String> {
        let re = Regex::new(r"[A-Za-z_]\w*|[^\w\s]|\d+|\s+").unwrap();
        re.find_iter(code).map(|m| m.as_str().to_string()).collect()
    }

    // ── helper: write content to a temp file and return its path ─────────────
    fn write_temp(content: &str) -> NamedTempFile {
        let f = NamedTempFile::new().unwrap();
        fs::write(f.path(), content).unwrap();
        f
    }

    #[test]
    fn test_prompt_and_response_differ_only_in_identifiers() {
        let obf_re = Regex::new(r"^(func_\d+|var_\d+)$").unwrap();

        let original = crate::sanitizer::sanitize(
            r#"public class TestClass100002 {
@Test public final void testGetTransmitStatusMessageNull() { XBeeTransmitStatus transmitStatus = null; TransmitException e = new TransmitException(transmitStatus); String result = e.getTransmitStatusMessage(); assertThat("Created 'TransmitException' does not return the expected transmit status message", result, is(nullValue(String.class))); }
}"#,
        );

        let obfuscated = crate::sanitizer::sanitize(
            r#"public class TestClass100002 {
@Test public final void func_1() { XBeeTransmitStatus var_1 = null; TransmitException var_2 = new TransmitException(var_1); String var_3 = var_2.getTransmitStatusMessage(); assertThat("Created 'TransmitException' does not return the expected transmit status message", var_3, is(nullValue(String.class))); }
}"#,
        );

        let p_toks = tokenize(&obfuscated);
        let r_toks = tokenize(&original);

        assert_eq!(
            p_toks.len(),
            r_toks.len(),
            "token count mismatch: prompt={} response={}",
            p_toks.len(),
            r_toks.len()
        );

        for (i, (pt, rt)) in p_toks.iter().zip(r_toks.iter()).enumerate() {
            let pt_s = pt.trim();
            let rt_s = rt.trim();
            if pt_s == rt_s {
                continue;
            }
            assert!(
                obf_re.is_match(pt_s),
                "token #{i}: prompt has non-obf difference {pt_s:?} vs {rt_s:?}"
            );
            assert!(
                !obf_re.is_match(rt_s),
                "token #{i}: response still contains obf token {rt_s:?}"
            );
        }
    }

    #[test]
    fn test_empty_obfuscated_file_returns_error() {
        let original = write_temp("public class T { @Test public void func_1() {} }");
        let obfuscated = write_temp(""); // ← empty: simulates the bug
        let out = NamedTempFile::new().unwrap();
        let out_path = format!("{}.jsonl", out.path().display());

        let result = super::generate_jsonl(
            original.path().to_str().unwrap(),
            obfuscated.path().to_str().unwrap(),
            &out_path,
        );

        assert!(
            result.is_err(),
            "generate_jsonl must fail when the obfuscated file is empty"
        );
        assert_eq!(
            result.unwrap_err().kind(),
            ErrorKind::InvalidData,
            "error kind must be InvalidData"
        );
        // The output file must not have been written (or must be empty/absent)
        // so it cannot silently corrupt the dataset.
        let written = fs::read_to_string(&out_path).unwrap_or_default();
        assert!(
            written.trim().is_empty(),
            "no JSONL must be written when obfuscated input is empty"
        );
    }

    #[test]
    fn test_obfuscated_file_is_sanitized() {
        // The obfuscated source contains a JSON unicode escape (\u0027 = apostrophe)
        // and an escaped single-quote (\'). sanitize() must fix both.
        let obfuscated_raw =
            "public class T { @Test public void func_1() { char var_1 = \\u0027A\\u0027; } }";
        let original_raw =
            "public class T { @Test public void testCharLiteral() { char c = 'A'; } }";

        let obfuscated_file = write_temp(obfuscated_raw);
        let original_file = write_temp(original_raw);
        let out = NamedTempFile::new().unwrap();
        let out_path = format!("{}.jsonl", out.path().display());

        super::generate_jsonl(
            original_file.path().to_str().unwrap(),
            obfuscated_file.path().to_str().unwrap(),
            &out_path,
        )
        .expect("generate_jsonl must succeed");

        let jsonl = fs::read_to_string(&out_path).unwrap();
        // After sanitise the unicode escape must have become a real apostrophe.
        assert!(
            jsonl.contains("'A'"),
            "unicode escape in obfuscated file must be resolved by sanitize; got: {jsonl}"
        );
        // The raw escape sequence must not appear in the output.
        assert!(
            !jsonl.contains("\\u0027"),
            "raw unicode escape must not survive into the JSONL"
        );
    }

    #[test]
    fn test_testclass95132_empty_obfuscated_file_is_rejected() {
        // The exact original source from the corrupt record.
        let original_source = concat!(
            "public class TestClass95132 {\n",
            "@Test public void testWithMoreData() throws Exception { ",
            "String text = \"def private String variable1='Hello world... from groovy'\"; ",
            "InputStream is = new ByteArrayInputStream(text.getBytes()); ",
            "GroovyASTModelBuilder b = new GroovyASTModelBuilder(is); ",
            "Model model = b.build(null); ",
            "Item[] items = model.getRoot().getChildren(); ",
            "assertEquals(1, items.length); ",
            "Item variableDefItem = items[0]; ",
            "assertEquals(\"VARIABLE_DEF\", variableDefItem.getName()); ",
            "int i = 0; ",
            "Item[] data = variableDefItem.getChildren(); ",
            "assertEquals(\"MODIFIERS\", data[i].getName()); ",
            "assertEquals(\"private\", data[i++].getChildren()[0].getName()); ",
            "assertEquals(\"TYPE\", data[i].getName()); ",
            "assertEquals(\"String\", data[i++].getChildren()[0].getName()); ",
            "assertEquals(\"variable1\", data[i++].getName()); ",
            "assertEquals(\"=\", data[i].getName()); ",
            "assertEquals(\"Hello world... from groovy\", data[i].getChildren()[0].getName()); }\n",
            "}"
        );

        let original_file = write_temp(original_source);
        // Simulate the bug: obfuscated file exists but is empty.
        let obfuscated_file = write_temp("");
        let out = NamedTempFile::new().unwrap();
        let out_path = format!("{}.jsonl", out.path().display());

        let result = super::generate_jsonl(
            original_file.path().to_str().unwrap(),
            obfuscated_file.path().to_str().unwrap(),
            &out_path,
        );

        assert!(
            result.is_err(),
            "generate_jsonl must reject an empty obfuscated file for TestClass95132"
        );
        assert_eq!(
            result.unwrap_err().kind(),
            ErrorKind::InvalidData,
            "error kind must be InvalidData, not a silent success"
        );

        // The corrupt record must NOT have been written.
        let written = fs::read_to_string(&out_path).unwrap_or_default();
        assert!(
            written.trim().is_empty(),
            "no JSONL must be written when the obfuscated input is empty; \
             a record with an empty prompt would corrupt the training dataset"
        );
    }

    #[test]
    fn test_testclass95132_valid_pair_is_written_correctly() {
        let original_source = concat!(
            "public class TestClass95132 {\n",
            "@Test public void testWithMoreData() throws Exception { ",
            "String text = \"def private String variable1='Hello world... from groovy'\"; ",
            "InputStream is = new ByteArrayInputStream(text.getBytes()); ",
            "GroovyASTModelBuilder b = new GroovyASTModelBuilder(is); ",
            "Model model = b.build(null); ",
            "Item[] items = model.getRoot().getChildren(); ",
            "assertEquals(1, items.length); ",
            "Item variableDefItem = items[0]; ",
            "assertEquals(\"VARIABLE_DEF\", variableDefItem.getName()); ",
            "int i = 0; ",
            "Item[] data = variableDefItem.getChildren(); ",
            "assertEquals(\"MODIFIERS\", data[i].getName()); ",
            "assertEquals(\"private\", data[i++].getChildren()[0].getName()); ",
            "assertEquals(\"TYPE\", data[i].getName()); ",
            "assertEquals(\"String\", data[i++].getChildren()[0].getName()); ",
            "assertEquals(\"variable1\", data[i++].getName()); ",
            "assertEquals(\"=\", data[i].getName()); ",
            "assertEquals(\"Hello world... from groovy\", data[i].getChildren()[0].getName()); }\n",
            "}"
        );

        // Obfuscated version: method → func_1, locals → var_1…var_6
        // (text, is, b, model, items, variableDefItem, i, data in declaration order)
        let obfuscated_source = concat!(
            "public class TestClass95132 {\n",
            "@Test public void func_1() throws Exception { ",
            "String var_1 = \"def private String variable1='Hello world... from groovy'\"; ",
            "InputStream var_2 = new ByteArrayInputStream(var_1.getBytes()); ",
            "GroovyASTModelBuilder var_3 = new GroovyASTModelBuilder(var_2); ",
            "Model var_4 = var_3.build(null); ",
            "Item[] var_5 = var_4.getRoot().getChildren(); ",
            "assertEquals(1, var_5.length); ",
            "Item var_6 = var_5[0]; ",
            "assertEquals(\"VARIABLE_DEF\", var_6.getName()); ",
            "int var_7 = 0; ",
            "Item[] var_8 = var_6.getChildren(); ",
            "assertEquals(\"MODIFIERS\", var_8[var_7].getName()); ",
            "assertEquals(\"private\", var_8[var_7++].getChildren()[0].getName()); ",
            "assertEquals(\"TYPE\", var_8[var_7].getName()); ",
            "assertEquals(\"String\", var_8[var_7++].getChildren()[0].getName()); ",
            "assertEquals(\"variable1\", var_8[var_7++].getName()); ",
            "assertEquals(\"=\", var_8[var_7].getName()); ",
            "assertEquals(\"Hello world... from groovy\", var_8[var_7].getChildren()[0].getName()); }\n",
            "}"
        );

        let original_file = write_temp(original_source);
        let obfuscated_file = write_temp(obfuscated_source);
        let out = NamedTempFile::new().unwrap();
        let out_path = format!("{}.jsonl", out.path().display());

        super::generate_jsonl(
            original_file.path().to_str().unwrap(),
            obfuscated_file.path().to_str().unwrap(),
            &out_path,
        )
        .expect("generate_jsonl must succeed with a valid obfuscated file");

        let jsonl = fs::read_to_string(&out_path).unwrap();
        let parsed: serde_json::Value =
            serde_json::from_str(jsonl.trim()).expect("output must be valid JSON");

        let prompt = parsed["prompt"].as_str().unwrap();
        let response = parsed["response"].as_str().unwrap();

        // Neither field may be empty.
        assert!(!prompt.is_empty(), "prompt must not be empty");
        assert!(!response.is_empty(), "response must not be empty");

        // Prompt must carry obfuscated names …
        assert!(
            prompt.contains("func_1"),
            "prompt must contain obfuscated method name func_1"
        );
        assert!(
            prompt.contains("var_1"),
            "prompt must contain obfuscated var_1 (text)"
        );
        assert!(
            prompt.contains("var_8"),
            "prompt must contain obfuscated var_8 (data)"
        );

        // … and must NOT contain the original identifier names.
        assert!(
            !prompt.contains("testWithMoreData"),
            "prompt must not expose original method name"
        );
        assert!(
            !prompt.contains("variableDefItem"),
            "prompt must not expose original var name"
        );

        // Response must carry the original names …
        assert!(
            response.contains("testWithMoreData"),
            "response must contain original method name"
        );
        assert!(
            response.contains("variableDefItem"),
            "response must contain original var name"
        );
        assert!(
            response.contains("text"),
            "response must contain original var 'text'"
        );
        assert!(
            response.contains("data"),
            "response must contain original var 'data'"
        );

        // … and must NOT contain obfuscated names.
        assert!(
            !response.contains("func_1"),
            "response must not contain obfuscated method name"
        );
        assert!(
            !response.contains("var_1"),
            "response must not contain obfuscated var names"
        );

        // String literals inside the source code must be preserved verbatim in both fields.
        assert!(
            prompt.contains("Hello world... from groovy"),
            "string literal must be preserved in prompt"
        );
        assert!(
            response.contains("Hello world... from groovy"),
            "string literal must be preserved in response"
        );
    }

    #[test]
    fn test_generate_jsonl_happy_path() {
        let original = write_temp(
            r#"public class TestClass100002 {
@Test public final void testGetTransmitStatusMessageNull() { String result = null; assertThat(result, is(nullValue(String.class))); }
}"#,
        );
        let obfuscated = write_temp(
            r#"public class TestClass100002 {
@Test public final void func_1() { String var_1 = null; assertThat(var_1, is(nullValue(String.class))); }
}"#,
        );
        let out = NamedTempFile::new().unwrap();
        let out_path = format!("{}.jsonl", out.path().display());

        super::generate_jsonl(
            original.path().to_str().unwrap(),
            obfuscated.path().to_str().unwrap(),
            &out_path,
        )
        .expect("generate_jsonl must succeed");

        let jsonl = fs::read_to_string(&out_path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(jsonl.trim()).unwrap();

        // prompt must be non-empty and contain obfuscated identifiers
        let prompt = parsed["prompt"].as_str().unwrap();
        assert!(!prompt.is_empty(), "prompt must not be empty");
        assert!(
            prompt.contains("func_1"),
            "prompt must contain obfuscated method name"
        );
        assert!(
            prompt.contains("var_1"),
            "prompt must contain obfuscated variable name"
        );

        // response must be non-empty and contain original identifiers
        let response = parsed["response"].as_str().unwrap();
        assert!(!response.is_empty(), "response must not be empty");
        assert!(
            response.contains("testGetTransmitStatusMessageNull"),
            "response must contain original method name"
        );
        assert!(
            response.contains("result"),
            "response must contain original variable name"
        );
    }

    /// Regression test for TestClass13026.
    ///
    /// The original dataset file has its string literals stored with extra
    /// layers of backslash escaping.  In the JSONL the prompt and response
    /// fields (after JSON decoding) contained these Java source strings:
    ///
    ///   prompt   setDelimiter arg: "\\"   (2 chars → 1 literal backslash)
    ///   response setDelimiter arg: "\\\\\\\\\\\\\\\\\\\\"  (10 chars → 5 literal backslashes, corrupt)
    ///
    /// This caused a token-count mismatch of 95 vs 111 that made the pair
    /// unusable.  generate_jsonl must silently repair it by copying the
    /// prompt's string literals into the response.
    ///
    /// The test strings below are the exact on-disk Java source bytes that
    /// produce those decoded JSONL values when read and serialised by serde_json.
    #[test]
    fn test_testclass13026_corrupt_string_literals_are_repaired() {
        // Obfuscated (prompt) — string literals are clean.
        // On disk the Java source contains:
        //   setDelimiter("\\")          ← 2 backslash chars
        //   findExactMatch("\\\\one\\\\two")  ← 4+4 backslash chars
        let obfuscated_source = "public class TestClass13026 {\n\
             @Test public void func_1() { \
             TreeDispatcher<String> var_1 = setupDispatcher(); \
             var_1.setDelimiter(\"\\\\\"); \
             assertEquals(dispatcher.findExactMatch(\"\\\\\\\\one\\\\\\\\two\"), \"/one/two\"); \
             assertNull(var_1.findExactMatch(\"/one/two\")); }\n\
             }";

        // Original (response) — real identifier names but corrupt string literals.
        // On disk the Java source contains:
        //   setDelimiter("\\\\\\\\\\")        ← 10 backslash chars (corrupt, odd-terminated)
        //   findExactMatch("\\\\\\\\one\\\\\\\\two")  ← 8+8 backslash chars (double-encoded)
        let original_source_corrupt = "public class TestClass13026 {\n\
             @Test public void testSetDelimiter() { \
             TreeDispatcher<String> dispatcher = setupDispatcher(); \
             dispatcher.setDelimiter(\"\\\\\\\\\\\\\\\\\\\\\"); \
             assertEquals(dispatcher.findExactMatch(\"\\\\\\\\\\\\\\\\one\\\\\\\\\\\\\\\\two\"), \"/one/two\"); \
             assertNull(dispatcher.findExactMatch(\"/one/two\")); }\n\
             }";

        let obfuscated_file = write_temp(obfuscated_source);
        let original_file = write_temp(original_source_corrupt);
        let out = NamedTempFile::new().unwrap();
        let out_path = format!("{}.jsonl", out.path().display());

        super::generate_jsonl(
            original_file.path().to_str().unwrap(),
            obfuscated_file.path().to_str().unwrap(),
            &out_path,
        )
        .expect("generate_jsonl must succeed: corrupt literals should be repaired");

        let jsonl = fs::read_to_string(&out_path).unwrap();
        let parsed: serde_json::Value =
            serde_json::from_str(jsonl.trim()).expect("output must be valid JSON");

        let prompt = parsed["prompt"].as_str().unwrap();
        let response = parsed["response"].as_str().unwrap();

        // Token counts must now be equal.
        let token_re = Regex::new(r"[A-Za-z_]\w*|[^\w\s]|\d+|\s+").unwrap();
        let p_count = token_re.find_iter(prompt).count();
        let r_count = token_re.find_iter(response).count();
        assert_eq!(
            p_count, r_count,
            "token counts must match after repair (prompt={p_count}, response={r_count})"
        );

        // Response must carry the original identifier names.
        assert!(
            response.contains("testSetDelimiter"),
            "response must contain original method name"
        );
        assert!(
            response.contains("dispatcher"),
            "response must contain original variable name"
        );

        // The repaired response must carry the same delimiter literal as the prompt.
        // After serde_json round-trips it, the 2-backslash Java source becomes "\\\\"
        // in the JSON string — exactly what the prompt contains.
        assert!(
            response.contains("\\\\"),
            "repaired response must contain the correct delimiter literal"
        );
    }

    // ── pipeline contract tests ──────────────────────────────────────────────

    /// Count double-quoted string literals using the same logic as
    /// `extract_string_literal_spans` / `fix_string_literals`.
    fn count_string_literals(src: &str) -> usize {
        let bytes = src.as_bytes();
        let mut count = 0;
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'"' {
                count += 1;
                i += 1;
                while i < bytes.len() {
                    match bytes[i] {
                        b'\\' => i += 2,
                        b'"' => {
                            i += 1;
                            break;
                        }
                        _ => i += 1,
                    }
                }
            } else {
                i += 1;
            }
        }
        count
    }

    /// Regression: TestClass9305 / TestClass10222.
    ///
    /// Dataset files contain `\\"` (double-backslash + quote) from a previous
    /// JSONL encoding pass.  Before the fix, `sanitized_original` kept the raw
    /// `\\"` sequences while `obfuscate_str` normalised them internally via
    /// `sanitize_backslashes`.  The two sides therefore had different string
    /// literal counts and `generate_jsonl_from_strings` failed with:
    ///   "String literal count mismatch between prompt and response"
    ///
    /// The fix: `main.rs` now calls `sanitize_backslashes` on the raw source
    /// before producing `sanitized_original`, so both sides start from the
    /// same normalised bytes.  This test encodes that contract: given the
    /// already-normalised `sanitized_original`, the obfuscated copy must have
    /// exactly the same number of string literals, and
    /// `generate_jsonl_from_strings` must succeed end-to-end.
    #[test]
    fn generate_jsonl_succeeds_for_double_escaped_backslash_quote() {
        // Simulate what main.rs produces after full_sanitize (both phases applied).
        // The \\" has already been collapsed to \" by sanitize_backslashes.
        let sanitized_original = concat!(
            "public class TestClass10222 {\n",
            "@Test public void should_load_spec() throws Exception {",
            " RestxSpec spec = new RestxSpecLoader(Factory.getInstance()).load(\"cases/test/test.spec.yaml\");",
            " assertThat(spec.getTitle()).isEqualTo(\"should say hello\");",
            " assertThat(spec.getWhens()).extracting(\"then\").extracting(\"expectedCode\", \"expected\")",
            " .containsExactly(Tuple.tuple(200, \"{\\\"message\\\":\\\"hello xavier, it's 14:33:18\\\"}\"));",
            " }\n}",
        );

        let obfuscated = crate::obfuscator::obfuscate_str(sanitized_original)
            .expect("obfuscate_str must not fail");

        // The core contract: both sides must have the same literal count.
        let original_count = count_string_literals(sanitized_original);
        let obfuscated_count = count_string_literals(&obfuscated);
        assert_eq!(
            original_count, obfuscated_count,
            "string literal count mismatch — original={} obfuscated={}\n\
             generate_jsonl_from_strings would fail with 'String literal count mismatch'",
            original_count, obfuscated_count
        );

        // End-to-end: generate_jsonl_from_strings must not return an error.
        let out = NamedTempFile::new().unwrap();
        let out_path = format!("{}.jsonl", out.path().display());
        generate_jsonl_from_strings(sanitized_original, &obfuscated, &out_path)
            .expect("generate_jsonl_from_strings must not fail with a literal count mismatch");

        let jsonl = fs::read_to_string(&out_path).unwrap();
        let parsed: serde_json::Value =
            serde_json::from_str(jsonl.trim()).expect("output must be valid JSON");
        let prompt = parsed["prompt"].as_str().unwrap();
        let response = parsed["response"].as_str().unwrap();

        assert_ne!(
            prompt, response,
            "prompt and response must differ — obfuscation must have renamed identifiers"
        );
        assert!(
            !prompt.contains("should_load_spec"),
            "prompt must not contain the original method name"
        );
        assert!(
            response.contains("should_load_spec"),
            "response must contain the original method name"
        );
    }

    /// Same contract for a plain file with no escaping issues — ensures the
    /// fix does not break the common case.
    #[test]
    fn generate_jsonl_literal_counts_match_plain_file() {
        let sanitized_original = concat!(
            "public class TestClass100002 {\n",
            "@Test public final void testGetTransmitStatusMessageNull() {",
            " XBeeTransmitStatus transmitStatus = null;",
            " TransmitException e = new TransmitException(transmitStatus);",
            " String result = e.getTransmitStatusMessage();",
            " assertThat(\"expected message\", result, is(nullValue(String.class)));",
            " }\n}",
        );

        let obfuscated = crate::obfuscator::obfuscate_str(sanitized_original)
            .expect("obfuscate_str must not fail");

        assert_eq!(
            count_string_literals(sanitized_original),
            count_string_literals(&obfuscated),
            "string literal count must match for a plain file"
        );

        let out = NamedTempFile::new().unwrap();
        let out_path = format!("{}.jsonl", out.path().display());
        generate_jsonl_from_strings(sanitized_original, &obfuscated, &out_path)
            .expect("generate_jsonl_from_strings must succeed for a plain file");
    }
}
