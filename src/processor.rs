use crate::sanitizer::sanitize;
use serde::Serialize;
use std::fs;
use std::fs::File;
use std::io::{BufWriter, Write};

#[derive(Serialize)]
struct PromptResponse {
    prompt: String,
    response: String,
}

pub fn generate_jsonl(
    original_file: &str,
    obfuscated_file: &str,
    output_file: &str,
) -> std::io::Result<()> {
    if !output_file.ends_with(".jsonl") {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Output file must have a .jsonl extension",
        ));
    }

    let original_code = sanitize(&fs::read_to_string(original_file)?);
    let obfuscated_code = sanitize(&fs::read_to_string(obfuscated_file)?);

    if obfuscated_code.trim().is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "Obfuscated file '{}' is empty or whitespace-only — \
                 the obfuscation step may not have run yet",
                obfuscated_file
            ),
        ));
    }
    if original_code.trim().is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "Original file '{}' is empty or whitespace-only",
                original_file
            ),
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

#[cfg(test)]
mod tests {
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
}
