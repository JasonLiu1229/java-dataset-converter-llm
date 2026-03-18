use crate::literal_blanker::blank_literals_permanently;
use crate::sanitizer::{sanitize_backslashes, sanitize_structural};
use serde::Serialize;
use std::fs;
use std::fs::File;
use std::io;
use std::io::{BufWriter, Write};

#[derive(Serialize)]
struct PromptResponse {
    prompt: String,
    response: String,
}

/// Write a JSONL training pair from in-memory source strings.
///
/// String literals are permanently replaced with `"_"` on both sides before
/// writing. `sanitize_backslashes` is then applied to clean up any residual
/// over-encoded backslash sequences in non-string tokens.
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

    let prompt = sanitize_backslashes(&blank_literals_permanently(obfuscated_src));
    let response = sanitize_backslashes(&blank_literals_permanently(original_src));

    if prompt.trim().is_empty() {
        return Err(std::io::Error::new(
            io::ErrorKind::InvalidData,
            "Obfuscated source is empty",
        ));
    }

    let mut writer = BufWriter::new(File::create(output_file)?);
    let pair = PromptResponse { prompt, response };
    writeln!(writer, "{}", serde_json::to_string(&pair)?)?;
    Ok(())
}

/// File-based wrapper: reads both files, applies `sanitize_structural`, then
/// delegates to `generate_jsonl_from_strings`.
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

    // ── helpers ──────────────────────────────────────────────────────────────

    fn tokenize(code: &str) -> Vec<String> {
        let re = Regex::new(r"[A-Za-z_]\w*|[^\w\s]|\d+|\s+").unwrap();
        re.find_iter(code).map(|m| m.as_str().to_string()).collect()
    }

    fn write_temp(content: &str) -> NamedTempFile {
        let f = NamedTempFile::new().unwrap();
        fs::write(f.path(), content).unwrap();
        f
    }

    // ── unit tests ────────────────────────────────────────────────────────────

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
        let obfuscated = write_temp("");
        let out = NamedTempFile::new().unwrap();
        let out_path = format!("{}.jsonl", out.path().display());

        let result = super::generate_jsonl(
            original.path().to_str().unwrap(),
            obfuscated.path().to_str().unwrap(),
            &out_path,
        );

        assert!(
            result.is_err(),
            "must fail when the obfuscated file is empty"
        );
        assert_eq!(result.unwrap_err().kind(), ErrorKind::InvalidData);

        let written = fs::read_to_string(&out_path).unwrap_or_default();
        assert!(
            written.trim().is_empty(),
            "no JSONL must be written on error"
        );
    }

    #[test]
    fn test_obfuscated_file_is_sanitized() {
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
        assert!(
            jsonl.contains("'X'"),
            "char literal must be replaced with dummy 'X'; got: {jsonl}"
        );
        assert!(
            !jsonl.contains("\\u0027"),
            "raw unicode escape must not survive"
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

        let prompt = parsed["prompt"].as_str().unwrap();
        assert!(!prompt.is_empty());
        assert!(
            prompt.contains("func_1"),
            "prompt must contain obfuscated method name"
        );
        assert!(
            prompt.contains("var_1"),
            "prompt must contain obfuscated variable name"
        );

        let response = parsed["response"].as_str().unwrap();
        assert!(!response.is_empty());
        assert!(
            response.contains("testGetTransmitStatusMessageNull"),
            "response must contain original method name"
        );
        assert!(
            response.contains("result"),
            "response must contain original variable name"
        );
    }

    // ── regression tests ──────────────────────────────────────────────────────

    /// Regression: TestClass95132 — empty obfuscated file must be rejected.
    #[test]
    fn test_testclass95132_empty_obfuscated_file_is_rejected() {
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
        let obfuscated_file = write_temp("");
        let out = NamedTempFile::new().unwrap();
        let out_path = format!("{}.jsonl", out.path().display());

        let result = super::generate_jsonl(
            original_file.path().to_str().unwrap(),
            obfuscated_file.path().to_str().unwrap(),
            &out_path,
        );

        assert!(result.is_err(), "must reject an empty obfuscated file");
        assert_eq!(result.unwrap_err().kind(), ErrorKind::InvalidData);

        let written = fs::read_to_string(&out_path).unwrap_or_default();
        assert!(
            written.trim().is_empty(),
            "no JSONL must be written on error"
        );
    }

    /// Regression: TestClass95132 — valid pair must produce correct JSONL.
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
        .expect("generate_jsonl must succeed");

        let jsonl = fs::read_to_string(&out_path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(jsonl.trim()).unwrap();

        let prompt = parsed["prompt"].as_str().unwrap();
        let response = parsed["response"].as_str().unwrap();

        assert!(!prompt.is_empty());
        assert!(!response.is_empty());

        assert!(
            prompt.contains("func_1"),
            "prompt must contain obfuscated method name"
        );
        assert!(
            prompt.contains("var_1"),
            "prompt must contain obfuscated var_1"
        );
        assert!(
            prompt.contains("var_8"),
            "prompt must contain obfuscated var_8"
        );
        assert!(
            !prompt.contains("testWithMoreData"),
            "prompt must not expose original method name"
        );
        assert!(
            !prompt.contains("variableDefItem"),
            "prompt must not expose original var name"
        );

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
        assert!(
            !response.contains("func_1"),
            "response must not contain obfuscated method name"
        );
        assert!(
            !response.contains("var_1"),
            "response must not contain obfuscated var names"
        );
        assert!(
            prompt.contains("\"_\"") && response.contains("\"_\""),
            "both sides must have dummy string literals"
        );
    }

    /// Regression: TestClass13026 — corrupt string literals in the original are
    /// no longer repaired (permanent blanking dropped string content entirely).
    /// The pair must still succeed and produce distinct prompt/response with
    /// correct identifier names.
    #[test]
    fn test_testclass13026_corrupt_string_literals_are_handled() {
        let obfuscated_source = "public class TestClass13026 {\n\
             @Test public void func_1() { \
             TreeDispatcher<String> var_1 = setupDispatcher(); \
             var_1.setDelimiter(\"\\\\\"); \
             assertEquals(dispatcher.findExactMatch(\"\\\\\\\\one\\\\\\\\two\"), \"/one/two\"); \
             assertNull(var_1.findExactMatch(\"/one/two\")); }\n\
             }";

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
        .expect("generate_jsonl must succeed");

        let jsonl = fs::read_to_string(&out_path).unwrap();
        let parsed: serde_json::Value =
            serde_json::from_str(jsonl.trim()).expect("output must be valid JSON");

        let prompt = parsed["prompt"].as_str().unwrap();
        let response = parsed["response"].as_str().unwrap();

        assert_ne!(prompt, response, "prompt and response must differ");
        assert!(
            response.contains("testSetDelimiter"),
            "response must contain original method name"
        );
        assert!(
            response.contains("dispatcher"),
            "response must contain original variable name"
        );
        assert!(
            !prompt.contains("testSetDelimiter"),
            "prompt must not contain original method name"
        );
        assert!(
            prompt.contains("\"_\"") && response.contains("\"_\""),
            "both sides must have dummy string literals"
        );
    }

    /// Regression: TestClass10222 — end-to-end pipeline via `obfuscate_str` +
    /// `generate_jsonl_from_strings` produces correct prompt/response pair.
    #[test]
    fn generate_jsonl_succeeds_for_double_escaped_backslash_quote() {
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

        let out = NamedTempFile::new().unwrap();
        let out_path = format!("{}.jsonl", out.path().display());
        generate_jsonl_from_strings(sanitized_original, &obfuscated, &out_path)
            .expect("generate_jsonl_from_strings must not fail");

        let jsonl = fs::read_to_string(&out_path).unwrap();
        let parsed: serde_json::Value =
            serde_json::from_str(jsonl.trim()).expect("output must be valid JSON");
        let prompt = parsed["prompt"].as_str().unwrap();
        let response = parsed["response"].as_str().unwrap();

        assert_ne!(prompt, response, "prompt and response must differ");
        assert!(
            !prompt.contains("should_load_spec"),
            "prompt must not contain original method name"
        );
        assert!(
            response.contains("should_load_spec"),
            "response must contain original method name"
        );
        assert!(
            prompt.contains("\"_\"") && response.contains("\"_\""),
            "both sides must have dummy string literals"
        );
    }

    /// Regression: plain file with no escaping issues must produce a valid pair.
    #[test]
    fn generate_jsonl_succeeds_for_plain_file() {
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

        let out = NamedTempFile::new().unwrap();
        let out_path = format!("{}.jsonl", out.path().display());
        generate_jsonl_from_strings(sanitized_original, &obfuscated, &out_path)
            .expect("generate_jsonl_from_strings must succeed for a plain file");

        let jsonl = fs::read_to_string(&out_path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(jsonl.trim()).unwrap();
        let prompt = parsed["prompt"].as_str().unwrap();
        let response = parsed["response"].as_str().unwrap();

        assert_ne!(prompt, response, "prompt and response must differ");
        assert!(response.contains("testGetTransmitStatusMessageNull"));
        assert!(!prompt.contains("testGetTransmitStatusMessageNull"));
        assert!(
            prompt.contains("\"_\"") && response.contains("\"_\""),
            "both sides must have dummy string literals"
        );
    }
}
