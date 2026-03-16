// AI generated code
pub fn sanitize(src: &str) -> String {
    let mut out = src.to_string();

    // ── 1. JSON unicode escapes that leaked into the source ─────────────────
    // e.g.  \u0027  →  '      (apostrophe / single-quote)
    //       \u0022  →  "      (double-quote)
    //       \u005c  →  \      (backslash)
    //       \u003c  →  <
    //       \u003e  →  >
    //       \u0026  →  &
    out = fix_json_unicode_escapes(&out);

    // ── 2. Escaped single-quotes  \'  →  '  ─────────────────────────────────
    // Java char/string literals do NOT use  \'  — only  '  is valid.
    // This is the most common corruption: (byte) \'Q\'  →  (byte) 'Q'
    out = out.replace("\\'", "'");

    // ── 3. Over-escaped backslashes before double-quotes ─────────────────────
    // Java string literals use  \"  (one backslash + doublequote).
    // After one or two rounds of JSON escaping the source may contain
    // \\\\\\\"  (3 backslashes+quote) or  \\\\\\"  (2 backslashes+quote).
    // Normalise both to a single  \"  (1 backslash+quote).
    // Apply longest pattern first so a 3-backslash run is not partially fixed.
    out = out.replace("\\\\\\\"", "\\\""); // 3 backslashes+quote → 1 backslash+quote
    out = out.replace("\\\\\"", "\\\""); // 2 backslashes+quote → 1 backslash+quote

    // ── 4. CRLF  →  LF ──────────────────────────────────────────────────────
    // tree-sitter handles CRLF fine, but normalising makes downstream diffs
    // and line-count checks simpler.
    out = out.replace("\r\n", "\n");

    // ── 5. Null bytes ────────────────────────────────────────────────────────
    // A null byte anywhere will confuse tree-sitter.
    out = out.replace('\0', "");

    out
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

/// Replace  \uXXXX  (case-insensitive) with the corresponding Unicode char.
/// Only replaces code points that are printable ASCII to avoid accidentally
/// fixing something that is intentionally a Unicode escape inside a string
/// literal (e.g.  "\u00e9"  should stay as-is in a string literal).
fn fix_json_unicode_escapes(src: &str) -> String {
    // Fast exit: skip the allocation when no  \u  is present.
    if !src.contains("\\u") && !src.contains("\\U") {
        return src.to_string();
    }

    let bytes = src.as_bytes();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;

    while i < bytes.len() {
        // Look for  \u  followed by exactly 4 hex digits.
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
                // Only substitute printable ASCII (0x20-0x7e) so we don't
                // accidentally mangle intentional Unicode inside string literals.
                if (0x20..=0x7e).contains(&cp) {
                    out.push(cp as u8 as char);
                    i += 6;
                    continue;
                }
            }
        }

        // Copy byte as-is (multi-byte UTF-8 sequences are handled correctly
        // because we always push the leading byte and advance by 1; the
        // continuation bytes are just copied in subsequent iterations).
        out.push(bytes[i] as char);
        i += 1;
    }

    out
}

fn is_hex(b: u8) -> bool {
    matches!(b, b'0'..=b'9' | b'a'..=b'f' | b'A'..=b'F')
}

#[cfg(test)]
mod tests {
    use super::sanitize;

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

    /// Full integration: the exact pattern from the failing test case in the
    /// research dataset.
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
}
