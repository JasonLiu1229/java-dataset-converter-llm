// AI generated code

/// Full sanitisation pipeline (steps 1–5).
///
/// Prefer calling [`sanitize_structural`] + [`fix_string_literals`] +
/// [`sanitize_backslashes`] separately when processing a (prompt, response)
/// pair, so that string-literal repair happens between the two backslash-
/// independent phases.  See `processor::generate_jsonl` for the correct order.
pub fn sanitize(src: &str) -> String {
    sanitize_backslashes(&sanitize_structural(src))
}

/// Phase 1 — structural fixes that are independent of backslash counts.
///
/// Steps applied:
/// 1. JSON unicode escapes  (`\u0022` → `"`, `\u0027` → `'`, …)
/// 2. Escaped single-quotes (`\'` → `'`)
/// 4. CRLF → LF
/// 5. Null bytes removed
///
/// Step 3 (backslash normalisation before `"`) is intentionally left out so
/// that [`fix_string_literals`] can compare literal contents before those
/// backslash runs are mutated.

pub fn sanitize_structural(src: &str) -> String {
    // ── Single pass: strip null bytes and collapse CRLF → LF ────────────────
    // This avoids two separate `.replace()` calls (each of which clones the
    // whole string). We write into a pre-allocated buffer and only allocate a
    // new String when the source actually contains one of these sequences.
    let needs_fixup = src.contains('\0') || src.contains("\r\n");
    let after_cr_null = if needs_fixup {
        let mut out = String::with_capacity(src.len());
        let mut bytes = src.as_bytes();
        while !bytes.is_empty() {
            match bytes[0] {
                b'\0' => {
                    bytes = &bytes[1..];
                }
                b'\r' if bytes.len() > 1 && bytes[1] == b'\n' => {
                    out.push('\n');
                    bytes = &bytes[2..];
                }
                _ => {
                    // Copy a full UTF-8 char at once.
                    let ch = src[src.len() - bytes.len()..]
                        .chars()
                        .next()
                        .unwrap_or('\0');
                    out.push(ch);
                    bytes = &bytes[ch.len_utf8()..];
                }
            }
        }
        out
    } else {
        src.to_string()
    };

    // ── 1. JSON unicode escapes that leaked into the source ──────────────────
    let after_unicode = fix_json_unicode_escapes(&after_cr_null);

    // ── 2. Escaped single-quotes  \'  →  '  ─────────────────────────────────
    fix_escaped_single_quotes(&after_unicode)
}

/// Phase 2 — over-escaped backslashes before double-quotes.
///
/// Step 3 of the full sanitisation pipeline:
/// Java string literals use `\"` (one backslash + double-quote).
/// After one or more rounds of JSON escaping the source may contain
/// arbitrarily deep backslash runs before a quote — e.g. `\\\\\"` (4
/// backslashes+quote from two encoding passes).  A single replacement
/// pass only reduces the run by 2–3 backslashes, leaving `\\"` which
/// still breaks `blank_literals` / `consume_string_literal`.
///
/// We therefore repeat the replacement until the output stabilises so
/// that every `\"` in the result has exactly one preceding backslash,
/// regardless of how many encoding passes the source went through.

pub fn sanitize_backslashes(src: &str) -> String {
    let mut out = src.to_string();
    loop {
        let next = out
            .replace("\\\\\\\"", "\\\"") // 3 backslashes+quote → 1 backslash+quote
            .replace("\\\\\"", "\\\""); // 2 backslashes+quote → 1 backslash+quote
        // Length check first — any replacement makes the string shorter,
        // so a length match means nothing changed (much cheaper than `==`).
        if next.len() == out.len() {
            break;
        }
        out = next;
    }
    out
}

// ---------------------------------------------------------------------------
// String literal repair
// ---------------------------------------------------------------------------

/// Extract all double-quoted string literal spans from Java source.
/// Returns a list of `(start, end)` byte offsets where `src[start..end]`
/// is the full literal including the surrounding `"` characters.
///
/// Uses the same corruption-recovery heuristic as `blank_literals` /
/// `consume_string_literal`: when an even backslash run precedes a `"` and
/// the following byte looks like it cannot start a valid Java token after a
/// string close (a letter, digit, `{`, `}`, `:`, `_`, `$`, or `\`), the `"`
/// is treated as an embedded escaped quote and scanning continues.
///
/// This must stay in sync with `literal_blanker::consume_string_literal` so
/// that `fix_string_literals` always counts the same number of literals as
/// `blank_literals`, preventing "String literal count mismatch" errors.
pub(crate) fn extract_string_literal_spans(src: &str) -> Vec<(usize, usize)> {
    fn is_suspicious_after_even_backslash_close(b: u8) -> bool {
        matches!(
            b,
            b'a'..=b'z'
                | b'A'..=b'Z'
                | b'0'..=b'9'
                | b'{'
                | b'}'
                | b':'
                | b','
                | b'['
                | b']'
                | b'\''  // apostrophe — never directly follows a Java string close
                | b'_'
                | b'$'
                | b'\\'
        )
    }

    let bytes = src.as_bytes();
    let mut spans = Vec::new();
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'"' {
            let start = i;
            i += 1;
            loop {
                if i >= bytes.len() {
                    break;
                }
                if bytes[i] == b'\\' {
                    // Count the full backslash run.
                    let run_start = i;
                    while i < bytes.len() && bytes[i] == b'\\' {
                        i += 1;
                    }
                    let run_len = i - run_start;
                    if i < bytes.len() && bytes[i] == b'"' {
                        let next = bytes.get(i + 1).copied().unwrap_or(b' ');
                        if run_len % 2 == 0 && is_suspicious_after_even_backslash_close(next) {
                            // Corruption heuristic: treat as embedded quote, keep scanning.
                            i += 1;
                        } else if run_len % 2 == 0 {
                            // Even run + non-suspicious next byte → real closing quote.
                            spans.push((start, i + 1));
                            i += 1;
                            break;
                        } else {
                            // Odd run → the last `\` escapes the `"`, keep scanning.
                            i += 1;
                        }
                    }
                } else if bytes[i] == b'"' {
                    // Bare unescaped closing quote.
                    spans.push((start, i + 1));
                    i += 1;
                    break;
                } else {
                    i += 1;
                }
            }
        } else {
            i += 1;
        }
    }

    spans
}

pub fn fix_string_literals(prompt: &str, response: &str) -> Option<String> {
    let p_spans = extract_string_literal_spans(prompt);
    let r_spans = extract_string_literal_spans(response);

    // If the counts differ the quote structure is broken beyond simple repair.
    if p_spans.len() != r_spans.len() {
        return None;
    }

    let mut result = response.to_string();

    // Iterate in reverse so that replacing a span does not shift the byte
    // offsets of spans that come earlier in the string.
    for ((r_start, r_end), (p_start, p_end)) in
        r_spans.iter().copied().zip(p_spans.iter().copied()).rev()
    {
        let r_inner = &response[r_start + 1..r_end - 1];
        let p_inner = &prompt[p_start + 1..p_end - 1];
        if r_inner != p_inner {
            result.replace_range(r_start + 1..r_end - 1, p_inner);
        }
    }

    Some(result)
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn fix_escaped_single_quotes(src: &str) -> String {
    let bytes = src.as_bytes();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'\\' {
            // Count the full run of consecutive backslashes starting at i.
            let run_start = i;
            while i < bytes.len() && bytes[i] == b'\\' {
                i += 1;
            }
            let num_backslashes = i - run_start;

            if i < bytes.len() && bytes[i] == b'\'' && num_backslashes % 2 == 1 {
                // Odd run ending before a quote: the last backslash is escaping
                // the quote (corruption artifact). Emit one fewer backslash and
                // a plain quote, consuming the quote too.
                for _ in 0..num_backslashes - 1 {
                    out.push('\\');
                }
                out.push('\'');
                i += 1; // skip the quote
            } else {
                // Even run, or not followed by a quote: emit all backslashes
                // verbatim; the loop will handle whatever follows normally.
                for _ in 0..num_backslashes {
                    out.push('\\');
                }
                // i already points past the backslash run; continue normally.
            }
        } else {
            let ch = src[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
        }
    }

    out
}

fn fix_json_unicode_escapes(src: &str) -> String {
    if !src.contains("\\u") && !src.contains("\\U") {
        return src.to_string();
    }

    let bytes = src.as_bytes();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;

    while i < bytes.len() {
        if i + 5 < bytes.len()
            && bytes[i] == b'\\'
            && (bytes[i + 1] == b'u' || bytes[i + 1] == b'U')
            && is_hex(bytes[i + 2])
            && is_hex(bytes[i + 3])
            && is_hex(bytes[i + 4])
            && is_hex(bytes[i + 5])
        {
            let hex = &src[i + 2..i + 6];
            if let Ok(cp) = u32::from_str_radix(hex, 16) {
                if (0x20..=0x7e).contains(&cp) {
                    out.push(cp as u8 as char);
                    i += 6;
                    continue;
                }
            }
        }

        let ch = src[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8(); // advance by the full char width, not just 1
    }

    out
}

fn is_hex(b: u8) -> bool {
    matches!(b, b'0'..=b'9' | b'a'..=b'f' | b'A'..=b'F')
}

#[cfg(test)]
mod tests {
    use super::{fix_string_literals, sanitize};

    #[test]
    fn test_escaped_single_quote_char_literal() {
        // (byte) \'Q\'  →  (byte) 'Q'
        let input = "Arrays.fill(buf, (byte) \\'Q\\');";
        let result = sanitize(input);
        assert_eq!(result, "Arrays.fill(buf, (byte) 'Q');");
    }

    #[test]
    fn test_escaped_single_quote_in_full_class() {
        let input = r#"public class T {
@Test public void m() {
    byte[] b = new byte[4];
    Arrays.fill(b, (byte) \'Q\');
}
}"#;
        let result = sanitize(input);
        assert!(result.contains("(byte) 'Q'"), "char literal must be fixed");
        assert!(
            !result.contains("\\'"),
            "no remaining escaped single-quotes"
        );
    }

    #[test]
    fn test_json_unicode_escape_apostrophe() {
        // \u0027  →  '
        let input = "char c = \\u0027A\\u0027;";
        let result = sanitize(input);
        assert_eq!(result, "char c = 'A';");
    }

    #[test]
    fn test_json_unicode_escape_double_quote() {
        // \u0022  →  "
        let input = "String s = \\u0022hello\\u0022;";
        let result = sanitize(input);
        assert_eq!(result, "String s = \"hello\";");
    }

    #[test]
    fn test_double_escaped_backslash_before_quote() {
        // \\\"  →  \"  (inside a Java string literal)
        let input = r#"String s = "{\\\"key\\\":\\\"val\\\"}";"#;
        let result = sanitize(input);
        assert_eq!(result, r#"String s = "{\"key\":\"val\"}";"#);
    }

    #[test]
    fn test_crlf_normalised() {
        let input = "public class T {\r\n  void m() {}\r\n}";
        let result = sanitize(input);
        assert!(!result.contains('\r'), "CRLF should become LF");
        assert!(result.contains('\n'), "LF must still be present");
    }

    #[test]
    fn test_null_bytes_removed() {
        let input = "public class T\0 { void m() {} }";
        let result = sanitize(input);
        assert!(!result.contains('\0'));
        assert!(result.contains("public class T"));
    }

    #[test]
    fn test_no_change_needed() {
        let input = "public class T { void m() { char c = 'Q'; } }";
        let result = sanitize(input);
        assert_eq!(result, input);
    }

    #[test]
    fn test_valid_backslash_in_string_unchanged() {
        // Plain \\ inside a string literal is valid Java; must not be touched.
        let input = r#"String path = "C:\\Users\\test";"#;
        let result = sanitize(input);
        assert_eq!(
            result, input,
            "valid Java double-backslash must be preserved"
        );
    }

    #[test]
    fn test_unicode_escape_only_printable_ascii() {
        // \u00e9 is 'é' — not printable ASCII, so must be left alone.
        let input = "String s = \"caf\\u00e9\";";
        let result = sanitize(input);
        assert_eq!(result, input, "non-ASCII unicode escape must be preserved");
    }

    #[test]
    fn test_failing_dataset_example() {
        let input = concat!(
            "public class TestClass109508 {\n",
            "@Test public void SearchStringFindsTooManyMatches() {\n",
            "    final int kTestSize = 1 << 20;\n",
            "    byte[] huge_dictionary = new byte[kTestSize];\n",
            "    Arrays.fill(huge_dictionary, (byte) \\'Q\\');\n",
            "    BlockHash.Match best_match = new BlockHash.Match();\n",
            "}\n",
            "}"
        );
        let result = sanitize(input);
        assert!(result.contains("(byte) 'Q'"), "char literal must be fixed");
        assert!(!result.contains("\\'"), "no escaped single-quotes remain");
        assert!(
            result.contains("huge_dictionary"),
            "identifiers must be preserved"
        );
        assert!(
            result.contains("BlockHash.Match"),
            "types must be preserved"
        );
    }

    #[test]
    fn test_multibyte_utf8_preserved() {
        // `Ã¢ÂÂ` is the UTF-8 bytes for `â` rendered as Latin-1 (mojibake).
        // The sanitizer must leave it byte-for-byte identical.
        let input = "String s = \"DataIdentification[Ã¢ÂÂDiscoveryMetadataÃ¢ÂÂ]\";";
        let result = sanitize(input);
        assert_eq!(
            result, input,
            "multi-byte UTF-8 / mojibake sequences must not be altered"
        );
    }

    #[test]
    fn test_unicode_escape_fix_does_not_disturb_mojibake() {
        let input = "String s = \\u0022Ã¢ÂÂhelloÃ¢ÂÂ\\u0022;";
        let result = sanitize(input);
        // The \u0022 escapes become real double-quotes; the mojibake is untouched.
        assert_eq!(result, "String s = \"Ã¢ÂÂhelloÃ¢ÂÂ\";");
    }

    #[test]
    fn test_real_unicode_chars_preserved() {
        let input = "String s = \"\u{2018}hello\u{2019}\";"; // ' hello '
        let result = sanitize(input);
        assert_eq!(result, input, "real Unicode chars must be preserved");
    }

    #[test]
    fn test_escaped_by_backslash_not_corrupted() {
        let input = r#"ESCAPED BY '\\' NULL DEFINED AS '\\N'"#;
        let result = sanitize(input);
        assert_eq!(
            result, input,
            "valid Java \\\\' (escaped backslash + quote) must not be altered"
        );
    }

    #[test]
    fn test_token_count_mismatch_regression() {
        let input = r#"ROW FORMAT DELIMITED FIELDS TERMINATED BY '\001' ESCAPED BY '\\' NULL DEFINED AS '\\N'"#;
        let result = sanitize(input);
        assert_eq!(
            result, input,
            "HiveHQL escape sequence must be preserved verbatim"
        );
    }

    #[test]
    fn test_simple_escaped_single_quote_unaffected() {
        let input = r#"Arrays.fill(buf, (byte) \'Q\');"#;
        let result = sanitize(input);
        assert_eq!(result, "Arrays.fill(buf, (byte) 'Q');");
    }

    // ── fix_string_literals tests ────────────────────────────────────────────

    #[test]
    fn test_fix_string_literals_no_corruption() {
        // When literals already match, the response must come back unchanged.
        let prompt = r#"public class T { @Test public void func_1() { String var_1 = "hello"; } }"#;
        let response =
            r#"public class T { @Test public void testFoo() { String greeting = "hello"; } }"#;
        let fixed = fix_string_literals(prompt, response).expect("should succeed");
        assert_eq!(fixed, response, "unchanged response must be returned as-is");
    }

    #[test]
    fn test_fix_string_literals_mismatched_count_returns_none() {
        // If the number of string literals differs the corruption is irrecoverable.
        let prompt = r#"String a = "one"; String b = "two";"#;
        let response = r#"String a = "one";"#; // missing second literal
        assert!(
            fix_string_literals(prompt, response).is_none(),
            "mismatched literal count must return None"
        );
    }

    #[test]
    fn test_fix_string_literals_only_identifiers_differ_after_fix() {
        let prompt = concat!(
            "public class TestClass13026 {\n",
            "@Test public void func_1() { ",
            "TreeDispatcher<String> var_1 = setupDispatcher(); ",
            "var_1.setDelimiter(\"\\\\\\\\\"); ",
            "assertEquals(dispatcher.findExactMatch(\"\\\\\\\\one\\\\\\\\two\"), \"/one/two\"); ",
            "assertNull(var_1.findExactMatch(\"/one/two\")); }\n",
            "}",
        );

        let response_corrupt = concat!(
            "public class TestClass13026 {\n",
            "@Test public void testSetDelimiter() { ",
            "TreeDispatcher<String> dispatcher = setupDispatcher(); ",
            "dispatcher.setDelimiter(\"\\\\\\\\\\\\\\\\\\\\\"); ",
            "assertEquals(dispatcher.findExactMatch(\"\\\\\\\\\\\\\\\\one\\\\\\\\\\\\\\\\two\"), \"/one/two\"); ",
            "assertNull(dispatcher.findExactMatch(\"/one/two\")); }\n",
            "}",
        );

        let fixed = fix_string_literals(prompt, response_corrupt)
            .expect("literal counts match — fix must succeed");

        let token_re = regex::Regex::new(r"[A-Za-z_]\w*|[^\w\s]|\d+|\s+").unwrap();
        let p_count = token_re.find_iter(prompt).count();
        let r_count = token_re.find_iter(&fixed).count();
        assert_eq!(
            p_count, r_count,
            "token counts must match after fix (prompt={p_count}, response={r_count})"
        );

        let obf_re = regex::Regex::new(r"^(func_\d+|var_\d+)$").unwrap();
        let p_toks: Vec<_> = token_re.find_iter(prompt).map(|m| m.as_str()).collect();
        let r_toks: Vec<_> = token_re.find_iter(&fixed).map(|m| m.as_str()).collect();
        for (i, (pt, rt)) in p_toks.iter().zip(r_toks.iter()).enumerate() {
            if pt == rt {
                continue;
            }
            assert!(
                obf_re.is_match(pt),
                "token #{i}: unexpected non-obf diff — prompt={pt:?} response={rt:?}"
            );
        }

        assert!(
            fixed.contains("\\\\\\\\"),
            "fixed response must contain the correct 4-backslash delimiter literal"
        );
        assert!(
            !fixed.contains("\\\\\\\\\\\\\\\\\\\\"),
            "fixed response must NOT contain the corrupt 10-backslash literal"
        );
    }

    // ── sanitize_backslashes multi-pass regression ───────────────────────────

    #[test]
    fn sanitize_backslashes_fully_reduces_four_backslash_run() {
        let input = "\\\\\\\\\""; // 4 backslashes + quote
        let result = super::sanitize_backslashes(input);
        assert_eq!(
            result, "\\\"",
            "4-backslash+quote must be fully reduced to \\\" in one call"
        );
    }

    #[test]
    fn sanitize_backslashes_fully_reduces_six_backslash_run() {
        let input = "\\\\\\\\\\\\\""; // 6 backslashes + quote
        let result = super::sanitize_backslashes(input);
        assert_eq!(
            result, "\\\"",
            "6-backslash+quote must be fully reduced to \\\" in one call"
        );
    }

    #[test]
    fn sanitize_backslashes_testclass11755_produces_correct_literal_count() {
        let raw = concat!(
            "public class TestClass11755 {\n",
            "@Test public void testString() throws IOException {",
            " final ByteArrayOutputStream baos = new ByteArrayOutputStream();",
            " final AJsonSerHelper ser = new AJsonSerHelper(baos);",
            " ser.writeStringLiteral(\"abcäöü\\\\r\\\\n\\\\t \\\\\\\\\\\"{}[]\");",
            " final String result = new String(baos.toByteArray(), \"utf-8\");",
            " assertEquals(\"\\\"abcäöü\\\\\\\\u000d\\\\\\\\u000a\\\\\\\\u0009 \\\\\\\\\\\\\\\\\\\\\\\\\\\"{}[]\\\"\", result);",
            " }\n}",
        );

        let result = super::sanitize_backslashes(raw);

        assert_eq!(
            super::sanitize_backslashes(&result),
            result,
            "sanitize_backslashes must be idempotent"
        );

        let bytes = result.as_bytes();
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
        assert_eq!(
            count, 3,
            "must find exactly 3 string literals after normalisation, found {}",
            count
        );

        assert!(result.contains("baos"), "identifier 'baos' must be visible");
        assert!(result.contains("ser"), "identifier 'ser' must be visible");
        assert!(
            result.contains("result"),
            "local var 'result' must be visible"
        );
    }

    // ── extract_string_literal_spans sync tests ──────────────────────────────

    fn smart_span_count(src: &str) -> usize {
        super::extract_string_literal_spans(src).len()
    }

    #[test]
    fn extract_spans_matches_blank_literals_for_corrupt_backslash_quote() {
        let src = concat!(
            "assertThat(httpRequest.body().trim())",
            ".isEqualTo(\"{\\\\\"message\\\\\":\\\\\"hello xavier, it's 14:33:18\\\\\"}\");",
        );
        assert_eq!(
            smart_span_count(src),
            1,
            "corrupt \\\\\" sequences must be counted as one literal"
        );
    }

    #[test]
    fn extract_spans_matches_blank_literals_for_valid_double_backslash() {
        let src = "assertThat(result).isEqualTo(\"\\\\\\\\\");";
        assert_eq!(
            smart_span_count(src),
            1,
            "valid \\\\\\\\ string must be one literal"
        );
    }

    #[test]
    fn extract_spans_count_equals_blank_literals_count_testclass10859() {
        let src = concat!(
            "assertTrue(res.getResponse().getContentAsString()",
            ".contains(\"\\\\\"activeProfiles\\\":[\\\\\"\"",
            "+profiles[0]+\"+\\\\\"]\"));",
        );
        let spans = super::extract_string_literal_spans(src);
        assert_eq!(
            spans.len(),
            3,
            "TestClass10859-style source must produce 3 literal spans, not more"
        );
    }
}
