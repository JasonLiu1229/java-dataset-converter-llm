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
    let obfuscated_code = fs::read_to_string(obfuscated_file)?;

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

    fn tokenize(code: &str) -> Vec<String> {
        let re = Regex::new(r"[A-Za-z_]\w*|[^\w\s]|\d+|\s+").unwrap();
        re.find_iter(code).map(|m| m.as_str().to_string()).collect()
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
}
