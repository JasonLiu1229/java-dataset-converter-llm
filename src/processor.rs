use crate::obfuscator::blank_source;
use crate::sanitizer::sanitize_structural;
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
/// Both sides have string/char literals permanently replaced with `"_"` /
/// `'X'` via `blank_source`, which mirrors `obfuscate_str`'s blanking logic:
/// it attempts `blank_literals_permanently` directly, then retries with
/// `sanitize_backslashes` if tree-sitter reports parse errors. This guarantees
/// both sides produce the same number of `"_"` literals even when the source
/// contains corrupt `\\"` sequences or valid strings ending in even backslash
/// runs like `"\\\\"`.
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

    let prompt = blank_source(obfuscated_src);
    let response = blank_source(original_src);

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

    /// Regression: TestClass12696 — valid Java string ending in even backslash run.
    ///
    /// The source contains `"\\\\"` (4 backslashes in the file = Java value `\\`).
    /// `sanitize_backslashes` collapses the trailing `\\\\"` pattern, destroying
    /// the string boundary and causing the blanker to produce garbage like
    /// `"_"%s"_"\\"_"` in the response. The fix: never run `sanitize_backslashes`
    /// before `blank_literals_permanently` — the blanker's even/odd heuristic
    /// handles both valid and corrupt patterns correctly on its own.
    #[test]
    fn test_testclass12696_valid_double_backslash_string_blanked_correctly() {
        // Java source: new TestPlaceholder("%s", "\\\\")
        // File bytes for second arg: "\\\\" = 4 backslashes + closing quote (valid Java, value = \\)
        let original_source = concat!(
            "public class TestClass12696 {\n",
            "@Test public void testProcessShouldHandleBackslashesCorrectly() {",
            " BasePlaceholder underTest = new TestPlaceholder(\"%s\", \"\\\\\\\\\");",
            " String result = underTest.process(\"%s\");",
            " assertThat(result).isEqualTo(\"\\\\\\\\\");",
            " }\n}",
        );
        let obfuscated_source = concat!(
            "public class TestClass12696 {\n",
            "@Test public void func_1() {",
            " BasePlaceholder var_1 = new TestPlaceholder(\"%s\", \"\\\\\\\\\");",
            " String var_2 = var_1.process(\"%s\");",
            " assertThat(var_2).isEqualTo(\"\\\\\\\\\");",
            " }\n}",
        );

        let out = NamedTempFile::new().unwrap();
        let out_path = format!("{}.jsonl", out.path().display());
        generate_jsonl_from_strings(original_source, obfuscated_source, &out_path)
            .expect("generate_jsonl_from_strings must not fail");

        let jsonl = fs::read_to_string(&out_path).unwrap();
        let parsed: serde_json::Value =
            serde_json::from_str(jsonl.trim()).expect("output must be valid JSON");

        let prompt = parsed["prompt"].as_str().unwrap();
        let response = parsed["response"].as_str().unwrap();

        // Both sides must have only clean dummy literals — no backslash garbage between them.
        assert!(
            prompt.contains("\"_\""),
            "prompt must contain dummy string literals"
        );
        assert!(
            response.contains("\"_\""),
            "response must contain dummy string literals"
        );
        assert!(
            !prompt.contains("\\\\"),
            "prompt must not contain raw backslash content outside a string: {prompt}"
        );
        assert!(
            !response.contains("\\\\"),
            "response must not contain raw backslash content outside a string: {response}"
        );

        // Identifiers must be correct on each side.
        assert!(
            prompt.contains("func_1"),
            "prompt must contain obfuscated method name"
        );
        assert!(
            response.contains("testProcessShouldHandleBackslashesCorrectly"),
            "response must contain original method name"
        );
        assert_ne!(prompt, response, "prompt and response must differ");
    }

    /// Regression: TestClass15410 — HTML string content leaking out of blanked literals.
    ///
    /// The original source has a long HTML string like `"<html>...<p>One</p>...</html>"`.
    /// The heuristic in `blank_literals_permanently` closes strings early when the byte
    /// after a `\\"` is non-suspicious (e.g. `>`, `<`, space). Since the HTML contains
    /// no backslash-quote sequences this shouldn't trigger — but the real issue is that
    /// without the parse-error fallback, ANY corrupt source that produces ERROR nodes
    /// goes undetected. `blank_source` uses tree-sitter to detect failures and retries,
    /// guaranteeing both sides have matching `"_"` counts.
    #[test]
    fn test_testclass15410_html_string_blanked_symmetrically() {
        let original_source = concat!(
            "public class TestClass15410 {\n",
            "@Test public void testClone() {",
            " Document doc = Jsoup.parse(\"<html><head><title>Hello</title></head><body><p>One</p><p>Two</p></body></html>\");",
            " Document clone = doc.clone();",
            " assertEquals(\"<html><head><title>Hello</title> </head><body><p>One</p><p>Two</p></body></html>\", TextUtil.stripNewlines(clone.html()));",
            " clone.title(\"Hello\");",
            " clone.select(\"p\").first().text(\"One more\").attr(\"id\", \"1\");",
            " assertEquals(\"<html><head><title>Hello</title></head><body><p id=\\\"1\\\">One more</p><p>Two</p></body></html>\", TextUtil.stripNewlines(clone.html()));",
            " assertEquals(\"<html><head><title>Hello</title> </head><body><p>One</p><p>Two</p></body></html>\", TextUtil.stripNewlines(doc.html()));",
            " }\n}",
        );
        let obfuscated_source = concat!(
            "public class TestClass15410 {\n",
            "@Test public void func_1() {",
            " Document var_1 = Jsoup.parse(\"<html><head><title>Hello</title></head><body><p>One</p><p>Two</p></body></html>\");",
            " Document var_2 = var_1.clone();",
            " assertEquals(\"<html><head><title>Hello</title> </head><body><p>One</p><p>Two</p></body></html>\", TextUtil.stripNewlines(var_2.html()));",
            " var_2.title(\"Hello\");",
            " var_2.select(\"p\").first().text(\"One more\").attr(\"id\", \"1\");",
            " assertEquals(\"<html><head><title>Hello</title></head><body><p id=\\\"1\\\">One more</p><p>Two</p></body></html>\", TextUtil.stripNewlines(var_2.html()));",
            " assertEquals(\"<html><head><title>Hello</title> </head><body><p>One</p><p>Two</p></body></html>\", TextUtil.stripNewlines(var_1.html()));",
            " }\n}",
        );

        let out = NamedTempFile::new().unwrap();
        let out_path = format!("{}.jsonl", out.path().display());
        generate_jsonl_from_strings(original_source, obfuscated_source, &out_path)
            .expect("generate_jsonl_from_strings must not fail");

        let jsonl = fs::read_to_string(&out_path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(jsonl.trim()).unwrap();
        let prompt = parsed["prompt"].as_str().unwrap();
        let response = parsed["response"].as_str().unwrap();

        // Count "_" occurrences — both sides must match.
        let prompt_count = prompt.matches("\"_\"").count();
        let response_count = response.matches("\"_\"").count();
        assert_eq!(
            prompt_count, response_count,
            "both sides must have the same number of dummy literals (prompt={prompt_count}, response={response_count})"
        );
        assert!(prompt_count > 0, "must have at least one blanked literal");

        // No raw HTML must survive outside a string.
        assert!(
            !response.contains("<html>"),
            "HTML content must not leak outside string boundaries in response"
        );
        assert!(
            !prompt.contains("<html>"),
            "HTML content must not leak outside string boundaries in prompt"
        );

        assert!(
            prompt.contains("func_1"),
            "prompt must contain obfuscated method name"
        );
        assert!(
            response.contains("testClone"),
            "response must contain original method name"
        );
        assert_ne!(prompt, response, "prompt and response must differ");
    }

    /// Regression: TestClass21169 — format string with `%s` after a corrupt `\\"` sequence.
    ///
    /// The original source contains `String.format("%s\"some text\"", ...)` where the
    /// string argument has `\\"` (two backslashes + bare quote) as a corruption artifact.
    /// The heuristic closes the string at `\\"` followed by `%` because `%` is not in
    /// the suspicious set, leaving `%s\\"_"` as raw tokens. `blank_source` detects the
    /// resulting parse errors and retries with `sanitize_backslashes`, producing clean
    /// `"_"` on both sides.
    #[test]
    fn test_testclass21169_format_string_with_corrupt_backslash_quote_blanked_correctly() {
        // The string argument "%s\\\"%s\\\"" simulates the corrupt pattern:
        // \\\\ = two backslashes in file, then \" = a bare quote (corrupt)
        let original_source = concat!(
            "public class TestClass21169 {\n",
            "@Test public void testGetExpressionVariableAsBooleanRequiredBlankValue() {",
            " Expression expression = mock(Expression.class);",
            " DelegateExecution execution = mock(DelegateExecution.class);",
            " when(expression.getValue(execution)).thenReturn(BLANK_TEXT);",
            " try {",
            " activitiHelper.getExpressionVariableAsBoolean(expression, execution, VARIABLE_NAME, VARIABLE_REQUIRED, NO_BOOLEAN_DEFAULT_VALUE);",
            " fail();",
            " } catch (IllegalArgumentException e) {",
            " assertEquals(String.format(\"%s\\\\\"variable '%s' is required\\\\\"\", VARIABLE_NAME), e.getMessage());",
            " } }\n}",
        );
        let obfuscated_source = concat!(
            "public class TestClass21169 {\n",
            "@Test public void func_1() {",
            " Expression var_1 = mock(Expression.class);",
            " DelegateExecution var_2 = mock(DelegateExecution.class);",
            " when(var_1.getValue(var_2)).thenReturn(BLANK_TEXT);",
            " try {",
            " activitiHelper.getExpressionVariableAsBoolean(var_1, var_2, VARIABLE_NAME, VARIABLE_REQUIRED, NO_BOOLEAN_DEFAULT_VALUE);",
            " fail();",
            " } catch (IllegalArgumentException e) {",
            " assertEquals(String.format(\"%s\\\\\"variable '%s' is required\\\\\"\", VARIABLE_NAME), e.getMessage());",
            " } }\n}",
        );

        let out = NamedTempFile::new().unwrap();
        let out_path = format!("{}.jsonl", out.path().display());
        generate_jsonl_from_strings(original_source, obfuscated_source, &out_path)
            .expect("generate_jsonl_from_strings must not fail");

        let jsonl = fs::read_to_string(&out_path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(jsonl.trim()).unwrap();
        let prompt = parsed["prompt"].as_str().unwrap();
        let response = parsed["response"].as_str().unwrap();

        let prompt_count = prompt.matches("\"_\"").count();
        let response_count = response.matches("\"_\"").count();
        assert_eq!(
            prompt_count, response_count,
            "both sides must have the same number of dummy literals (prompt={prompt_count}, response={response_count})"
        );
        assert!(prompt_count > 0, "must have at least one blanked literal");

        // No format specifier or corrupt backslash sequence must leak outside a string.
        assert!(
            !response.contains("%s\\"),
            "corrupt backslash-quote must not leak outside string in response"
        );
        assert!(
            !prompt.contains("%s\\"),
            "corrupt backslash-quote must not leak outside string in prompt"
        );

        assert!(
            prompt.contains("func_1"),
            "prompt must contain obfuscated method name"
        );
        assert!(
            response.contains("testGetExpressionVariableAsBooleanRequiredBlankValue"),
            "response must contain original method name"
        );
        assert_ne!(prompt, response, "prompt and response must differ");
    }
}
