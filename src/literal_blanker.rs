#[derive(Debug, Clone, Default)]
pub struct LiteralStore {
    entries: Vec<LiteralEntry>,
}

#[derive(Debug, Clone)]
struct LiteralEntry {
    placeholder: String,
    original: String,
}

// ---------------------------------------------------------------------------
// blank_literals
// ---------------------------------------------------------------------------

pub fn blank_literals_permanently(src: &str) -> String {
    let bytes = src.as_bytes();
    let len = bytes.len();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;

    while i < len {
        // line comment
        if i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'/' {
            let start = i;
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            out.push_str(&src[start..i]);
            continue;
        }
        // block comment
        if i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            let start = i;
            i += 2;
            while i + 1 < len && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            if i + 1 < len {
                i += 2;
            }
            out.push_str(&src[start..i]);
            continue;
        }
        // string literal — consume it entirely, emit dummy
        if bytes[i] == b'"' {
            let (_literal, end) = consume_string_literal(bytes, i);
            out.push_str("\"_\"");
            i = end;
            continue;
        }
        // char literal
        if bytes[i] == b'\'' {
            if let Some((_literal, end)) = try_consume_char_literal(bytes, i) {
                out.push_str("'X'");
                i = end;
                continue;
            }
        }
        let ch = src[i..].chars().next().unwrap_or('\0');
        out.push(ch);
        i += ch.len_utf8();
    }

    out
}

/// Replace every string / char literal in `src` with a stable placeholder.
///
/// Returns `(blanked_source, store)`.
///
/// Guarantees:
/// * The blanked source is structurally valid Java (tree-sitter can parse it).
/// * Placeholders are assigned in left-to-right source order.
/// * Literals inside `//` and `/* */` comments are **not** blanked.
pub fn blank_literals(src: &str) -> (String, LiteralStore) {
    let bytes = src.as_bytes();
    let len = bytes.len();

    let mut out = String::with_capacity(src.len());
    let mut store = LiteralStore::default();
    let mut i = 0;

    while i < len {
        // ── line comment  // … \n ───────────────────────────────────────────
        if i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'/' {
            let start = i;
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            out.push_str(&src[start..i]);
            continue;
        }

        // ── block comment  /* … */ ──────────────────────────────────────────
        if i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            let start = i;
            i += 2;
            while i + 1 < len && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            if i + 1 < len {
                i += 2; // consume '*/'
            }
            out.push_str(&src[start..i]);
            continue;
        }

        // ── string literal  "…" ────────────────────────────────────────────
        if bytes[i] == b'"' {
            let (literal, end) = consume_string_literal(bytes, i);
            let idx = store.entries.len();
            let placeholder = format!("\"STR_{}\"", idx);
            store.entries.push(LiteralEntry {
                placeholder: placeholder.clone(),
                original: literal,
            });
            out.push_str(&placeholder);
            i = end;
            continue;
        }

        // ── char literal  '…' ──────────────────────────────────────────────
        if bytes[i] == b'\'' {
            if let Some((literal, end)) = try_consume_char_literal(bytes, i) {
                let placeholder = "'X'".to_string();
                store.entries.push(LiteralEntry {
                    placeholder: placeholder.clone(),
                    original: literal,
                });
                out.push_str(&placeholder);
                i = end;
                continue;
            }
        }

        // ── verbatim passthrough ────────────────────────────────────────────
        let ch = src[i..].chars().next().unwrap_or('\0');
        out.push(ch);
        i += ch.len_utf8();
    }

    (out, store)
}

// ---------------------------------------------------------------------------
// restore_literals
// ---------------------------------------------------------------------------

/// Replace every placeholder inserted by [`blank_literals`] with its original
/// literal, restoring the source to its pre-blanked form.
///
/// Optimisation: instead of repeatedly calling `find()` + `replace_range()`
/// on a mutable `String` (O(n × m) total), this does a single left-to-right
/// scan that builds the output in one pass (O(n + total_literal_bytes)).
/// Placeholders are guaranteed to appear in insertion order (left-to-right),
/// so we can advance `rest` forward without back-tracking.
pub fn restore_literals(blanked: &str, store: &LiteralStore) -> String {
    if store.entries.is_empty() {
        return blanked.to_string();
    }

    // Pre-size the output: it will be at least as long as `blanked` because
    // original literals are usually longer than their placeholders.
    let mut result = String::with_capacity(blanked.len());
    let mut rest = blanked;

    for entry in &store.entries {
        match rest.find(&entry.placeholder) {
            Some(pos) => {
                // Push everything before the placeholder verbatim, then the
                // original literal, then advance past the placeholder.
                result.push_str(&rest[..pos]);
                result.push_str(&entry.original);
                rest = &rest[pos + entry.placeholder.len()..];
            }
            None => {
                // A missing placeholder means the processing step deleted the
                // literal — caller bug. Skip silently so the rest is still
                // restored.
            }
        }
    }

    // Append whatever remains after the last placeholder.
    result.push_str(rest);
    result
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Returns `true` for bytes that cannot validly appear immediately after a
/// closing `"` in well-formed Java source.
///
/// This is used as a look-ahead heuristic inside [`consume_string_literal`]:
/// when we encounter an even run of backslashes followed by `"`, that `"` is
/// normally the closing quote.  But in dataset files that have been through one
/// or more JSONL encoding passes, each `\"` inside a string was doubled to `\\"`.
/// When `consume_string_literal` hits that `\\"`, it sees an even backslash run
/// + `"` and would incorrectly close the string, leaving the rest of the content
/// (e.g. `message\":\"val\"}`) as raw tokens outside any string — making the
/// blanked source unparseable by tree-sitter.
///
/// The heuristic: if the character immediately following the candidate closing
/// `"` is one that **cannot** start a valid Java token after a string literal
/// (a letter, digit, or any JSON structural character: `{`, `}`, `:`, `,`,
/// `[`, `]`, `_`, `$`, or `\`), the close is almost certainly spurious
/// corruption and we treat the `"` as an embedded escaped quote instead.
///
/// Note: this lookahead fires **only** for backslash-preceded `"` (even run).
/// A bare unescaped `"` always closes the string unconditionally.
fn is_suspicious_after_even_backslash_close(b: u8) -> bool {
    matches!(
        b,
        b'a'..=b'z'     // letter — identifier or JSON key/value
        | b'A'..=b'Z'
        | b'0'..=b'9'   // digit
        | b'{'          // JSON object open
        | b'}'          // JSON object close (safe to add: only fires for backslash-preceded ")
        | b':'          // JSON key-value separator
        | b','          // JSON array/object separator
        | b'['          // JSON array open
        | b']'          // JSON array close
        | b'\''         // apostrophe — never directly follows a Java string close
        | b'_'          // identifier char
        | b'$'          // identifier char
        | b'\\'         // another backslash run follows
    )
}

/// Consume a double-quoted Java string literal starting at byte offset `start`.
///
/// Returns `(full_literal_text, one_past_closing_quote)`.
///
/// Handles standard Java escape sequences (`\"`, `\\`, etc.) and also applies
/// a corruption-recovery heuristic for dataset files that contain `\\"` where
/// they should contain `\"`: when an even backslash run precedes a `"` and the
/// byte immediately after that `"` looks like it could not start a valid Java
/// token after a string close, the `"` is treated as an embedded escaped quote
/// and scanning continues.
///
/// Does **not** handle Java 15+ text blocks (triple-quote).
fn consume_string_literal(bytes: &[u8], start: usize) -> (String, usize) {
    debug_assert_eq!(bytes[start], b'"');
    let mut i = start + 1;

    while i < bytes.len() {
        match bytes[i] {
            b'\\' => {
                // Count the full run of consecutive backslashes.
                let run_start = i;
                while i < bytes.len() && bytes[i] == b'\\' {
                    i += 1;
                }
                let run_len = i - run_start;

                if i < bytes.len() && bytes[i] == b'"' {
                    if run_len % 2 == 0 {
                        // Even run: the backslashes are all paired, so the `"`
                        // is a candidate closing quote.
                        let next = bytes.get(i + 1).copied().unwrap_or(b' ');
                        if is_suspicious_after_even_backslash_close(next) {
                            // Heuristic: looks like a corrupt embedded quote.
                            // Keep scanning — do NOT close the string here.
                            i += 1; // skip the `"`
                        } else {
                            // Real closing quote.
                            i += 1; // skip the `"`
                            break;
                        }
                    } else {
                        // Odd run: the last `\` escapes the `"` — it is an
                        // embedded quoted character, not the closing quote.
                        i += 1; // skip the `"`
                    }
                }
                // If not followed by `"`, the backslash run is just part of
                // the string content (e.g. `\\n`, `\\t`, …); continue.
            }
            b'"' => {
                // Bare unescaped closing quote — always terminates.
                i += 1;
                break;
            }
            _ => {
                i += 1;
            }
        }
    }

    // CORRECTNESS: We must decode the byte slice as UTF-8, not byte-by-byte via
    // `b as char` (Latin-1), which would corrupt any multi-byte character
    // (e.g. `é` → `Ã©`) and cause token-count mismatches in the JSONL pairs.
    (String::from_utf8_lossy(&bytes[start..i]).into_owned(), i)
}

/// Try to consume a char literal starting at `start`.
///
/// Returns `(literal_text, one_past_closing_quote)` on success, or `None` if
/// the bytes at `start` do not look like a valid Java char literal.
fn try_consume_char_literal(bytes: &[u8], start: usize) -> Option<(String, usize)> {
    debug_assert_eq!(bytes[start], b'\'');

    // Minimum char literal: 'X' → 3 bytes.
    if start + 2 >= bytes.len() {
        return None;
    }

    let mut i = start + 1;

    // Escape sequence
    if bytes[i] == b'\\' {
        i += 1; // skip the backslash
        if i >= bytes.len() {
            return None;
        }
        i += 1; // skip the escaped char
    } else {
        // Ordinary character — must not be a newline or another `'`.
        if bytes[i] == b'\n' || bytes[i] == b'\'' {
            return None;
        }
        // Advance past the UTF-8 char (may be multi-byte).
        // We work at byte level so just skip one byte here; for ASCII this is
        // correct. For multi-byte chars the closing `'` will still be found.
        i += 1;
    }

    // Closing quote
    if i >= bytes.len() || bytes[i] != b'\'' {
        return None;
    }
    i += 1;

    // CORRECTNESS: decode as UTF-8, not byte-by-byte Latin-1 (`b as char`).
    Some((String::from_utf8_lossy(&bytes[start..i]).into_owned(), i))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(src: &str) -> String {
        let (blanked, store) = blank_literals(src);
        restore_literals(&blanked, &store)
    }

    #[test]
    fn round_trip_simple_string() {
        let src = r#"String s = "hello world";"#;
        assert_eq!(round_trip(src), src);
    }

    #[test]
    fn round_trip_escaped_quote_in_string() {
        let src = r#"String s = "say \"hi\"";"#;
        assert_eq!(round_trip(src), src);
    }

    #[test]
    fn round_trip_char_plain() {
        assert_eq!(round_trip("char c = 'A';"), "char c = 'A';");
    }

    #[test]
    fn round_trip_char_escaped_single_quote() {
        let src = "char c = '\\'';";
        assert_eq!(round_trip(src), src);
    }

    #[test]
    fn round_trip_char_escaped_backslash() {
        assert_eq!(round_trip(r"char c = '\\';"), r"char c = '\\';");
    }

    #[test]
    fn round_trip_multiple_strings() {
        let src = r#"assertEquals("hello", "world");"#;
        assert_eq!(round_trip(src), src);
    }

    #[test]
    fn round_trip_testclass10186() {
        let src = r#"assertThat(httpRequest.body().trim()).isEqualTo("{\"message\":\"hello xavier, it's 14:33:18\"}");"#;
        assert_eq!(round_trip(src), src);
    }

    #[test]
    fn round_trip_apostrophe_in_string_not_a_char_literal() {
        let src = r#"isEqualTo("it's fine");"#;
        assert_eq!(round_trip(src), src);
    }

    #[test]
    fn round_trip_is_char_escaped_single_quote() {
        // is('\'') — TestClass50918-style
        let src = "assertThat(content.charAt(2), is('\\''));";
        assert_eq!(round_trip(src), src);
    }

    #[test]
    fn line_comment_not_blanked() {
        let src = "int x = 1; // don't touch \"this\"";
        let (blanked, _) = blank_literals(src);
        assert!(blanked.contains("\"this\""));
    }

    #[test]
    fn block_comment_not_blanked() {
        let src = "/* \"keep me\" */ int x = 1;";
        let (blanked, _) = blank_literals(src);
        assert!(blanked.contains("\"keep me\""));
    }

    #[test]
    fn round_trip_unicode_escape_char_literal() {
        let src = "char c = '\\u0041';";
        assert_eq!(round_trip(src), src);
    }

    #[test]
    fn round_trip_heavy_backslash_string() {
        let src = r#"String url = "jdbc:h2:file:path\\\\upstream\\\\modeshape";"#;
        assert_eq!(round_trip(src), src);
    }

    #[test]
    fn round_trip_single_quotes_inside_string() {
        let src = r#"String content = "--'this is a single-quoted \\n string'-";"#;
        assert_eq!(round_trip(src), src);
    }

    #[test]
    fn blanked_has_no_escaped_quotes() {
        let src = r#"assertThat(s).isEqualTo("{\"key\":\"val\"}");"#;
        let (blanked, _) = blank_literals(src);
        assert!(!blanked.contains("\\\""));
    }

    #[test]
    fn blanked_char_is_plain_x() {
        let src = "char c = '\\'';";
        let (blanked, _) = blank_literals(src);
        assert!(blanked.contains("'X'"));
        assert!(!blanked.contains("\\'"));
    }

    #[test]
    fn placeholder_count_matches_literal_count() {
        let src = r#"assertEquals("a", "b", "c");"#;
        let (blanked, store) = blank_literals(src);
        assert_eq!(store.entries.len(), 3);
        assert!(blanked.contains("\"STR_0\""));
        assert!(blanked.contains("\"STR_1\""));
        assert!(blanked.contains("\"STR_2\""));
    }

    #[test]
    fn identifiers_survive_blank_restore() {
        let src = r#"public class T {
    @Test public void should_use_spec() throws Exception {
        WebServer server = SpecsServer.getServer(WebServers.findAvailablePort(), "/api", ".");
        server.start();
        HttpRequest httpRequest = HttpRequest.get(server.baseUrl() + "/api/message?who=xavier");
        assertThat(httpRequest.code()).isEqualTo(200);
        assertThat(httpRequest.body().trim()).isEqualTo("{\"message\":\"hello xavier, it's 14:33:18\"}");
        server.stop();
    }
}"#;
        assert_eq!(round_trip(src), src);
        let (blanked, _) = blank_literals(src);
        assert!(blanked.contains("server"));
        assert!(blanked.contains("httpRequest"));
        assert!(!blanked.contains("xavier"));
        assert!(!blanked.contains("14:33:18"));
    }

    #[test]
    fn empty_string_literal() {
        let src = r#"String s = "";"#;
        assert_eq!(round_trip(src), src);
    }

    #[test]
    fn no_literals() {
        let src = "int x = 1 + 2;";
        let (blanked, store) = blank_literals(src);
        assert_eq!(blanked, src);
        assert!(store.entries.is_empty());
    }

    #[test]
    fn consecutive_string_literals() {
        let src = r#"f("a" + "b" + "c")"#;
        assert_eq!(round_trip(src), src);
    }

    // ── regression tests ────────────────────────────────────────────────────

    #[test]
    fn blanked_form_is_clean_testclass10222() {
        let src = concat!(
            "public class TestClass10222 {\n",
            "@Test public void should_load_spec() throws Exception {",
            " RestxSpec spec = new RestxSpecLoader(Factory.getInstance()).load(\"cases/test/test.spec.yaml\");",
            " assertThat(spec.getWhens()).extracting(\"then\").extracting(\"expectedCode\", \"expected\")",
            " .containsExactly(Tuple.tuple(200, \"{\\\"message\\\":\\\"hello xavier, it's 14:33:18\\\"}\"));",
            " }\n}",
        );
        let (blanked, store) = blank_literals(src);

        assert!(
            !blanked.contains("\\\""),
            "no escaped quotes must remain in blanked source"
        );
        assert!(
            !blanked.contains("it's"),
            "apostrophe inside string must be blanked"
        );
        assert!(
            blanked.contains("spec"),
            "local identifier 'spec' must survive blanking"
        );
        assert!(
            blanked.contains("should_load_spec"),
            "method name must survive blanking"
        );
        assert_eq!(
            restore_literals(&blanked, &store),
            src,
            "round-trip must be lossless"
        );
    }

    #[test]
    fn blanked_form_is_clean_testclass10150() {
        let src = concat!(
            "public class TestClass10150 {\n",
            "@Test public void rot13() {",
            " Provisioner p = new Provisioner();",
            " String result = p.rot13(\"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ1234567890{}\\\"':=\");",
            " Assert.assertEquals(\"nopqrstuvwxyzabcdefghijklmNOPQRSTUVWXYZABCDEFGHIJKLM1234567890{}\\\"':=\", result);",
            " }\n}",
        );
        let (blanked, store) = blank_literals(src);

        assert!(!blanked.contains("\\\""), "no escaped quotes must remain");
        assert!(
            !blanked.contains("':="),
            "the problematic tail must be inside a placeholder"
        );
        assert!(blanked.contains("p"), "local identifier 'p' must survive");
        assert!(
            blanked.contains("result"),
            "local identifier 'result' must survive"
        );
        assert_eq!(store.entries.len(), 2, "exactly two string literals");
        assert_eq!(
            restore_literals(&blanked, &store),
            src,
            "round-trip must be lossless"
        );
    }

    #[test]
    fn blanked_form_is_clean_testclass10693() {
        let src = concat!(
            "public class TestClass10693 {\n",
            "@Test(expected = IllegalArgumentException.class)",
            " public void testWrongPath() {",
            " Path c = new Path(\"root.\\\"sg\\\".\\\"d1\\\".\\\"s1\\\"\\\"\", true);",
            " System.out.println(c.getMeasurement());",
            " }\n}",
        );
        let (blanked, store) = blank_literals(src);

        assert!(!blanked.contains("\\\""), "no escaped quotes must remain");
        assert!(blanked.contains("c"), "local identifier 'c' must survive");
        assert_eq!(
            restore_literals(&blanked, &store),
            src,
            "round-trip must be lossless"
        );
    }

    #[test]
    fn blanked_form_is_clean_testclass20926() {
        let src = concat!(
            "public class TestClass20926 {\n",
            "@Test void testQuoted() {",
            " QuotedQualityCSV values = new QuotedQualityCSV();",
            " values.addValue(\" value 0.5 ; p = \\\"v ; q = \\\\\\\"0.5\\\\\\\" , value 1.0 \\\" \");",
            " assertContains(values.getValues(), \"value 0.5;p=\\\"v ; q = \\\\\\\"0.5\\\\\\\" , value 1.0 \\\"\");",
            " }\n}",
        );
        let (blanked, store) = blank_literals(src);

        assert!(!blanked.contains("\\\""), "no escaped quotes must remain");
        assert!(
            blanked.contains("values"),
            "local identifier 'values' must survive"
        );
        assert_eq!(store.entries.len(), 2, "exactly two string literals");
        assert_eq!(
            restore_literals(&blanked, &store),
            src,
            "round-trip must be lossless"
        );
    }

    #[test]
    fn blanked_form_is_clean_testclass74779() {
        let src = concat!(
            "public class TestClass74779 {\n",
            "@Test public void escapeQuote() {",
            " final String escapedQuote = underTest.apply((int) '\"');",
            " assertThat(escapedQuote).isEqualTo(\"\\\\\\\"\");",
            " }\n}",
        );
        let (blanked, store) = blank_literals(src);

        assert!(!blanked.contains("\\\""), "no escaped quotes must remain");
        assert!(
            blanked.contains("escapedQuote"),
            "local identifier 'escapedQuote' must survive"
        );
        assert!(
            blanked.contains("'X'"),
            "char literal '\"' must be replaced with 'X'"
        );
        assert_eq!(
            restore_literals(&blanked, &store),
            src,
            "round-trip must be lossless"
        );
    }

    #[test]
    fn blanked_form_is_clean_entity_tag_strings() {
        let src = concat!(
            "public class TestClass8595 {\n",
            "@Test public void parsesStringQuoted() {",
            " EntityTag entityTag = entityTagHeaderDelegate.fromString(\"\\\"test \\\\\\\"test\\\\\\\"\\\"\");",
            " assertFalse(entityTag.isWeak());",
            " assertEquals(\"test \\\"test\\\"\", entityTag.getValue());",
            " }\n}",
        );
        let (blanked, store) = blank_literals(src);

        assert!(!blanked.contains("\\\""), "no escaped quotes must remain");
        assert!(
            blanked.contains("entityTag"),
            "local identifier 'entityTag' must survive"
        );
        assert_eq!(store.entries.len(), 2, "exactly two string literals");
        assert_eq!(
            restore_literals(&blanked, &store),
            src,
            "round-trip must be lossless"
        );
    }

    #[test]
    fn blanked_form_is_clean_single_double_quote_string() {
        let src = concat!(
            "public class TestClass67764 {\n",
            "@Test public void testQuoteCharacterUsedJustForComplexEnumEnd() {",
            " final String[] split = ListSplitter.split(\"\\\"\", true, \"Prague, Boston, \\\"Helsinki, Finland\\\"\");",
            " assertEquals(3, split.length);",
            " assertEquals(\"Prague\", split[0]);",
            " }\n}",
        );
        let (blanked, store) = blank_literals(src);

        assert!(!blanked.contains("\\\""), "no escaped quotes must remain");
        assert!(
            blanked.contains("split"),
            "local identifier 'split' must survive"
        );
        assert_eq!(store.entries.len(), 3, "exactly three string literals");
        assert_eq!(
            restore_literals(&blanked, &store),
            src,
            "round-trip must be lossless"
        );
    }

    #[test]
    fn blanked_does_not_consume_identifiers_into_string() {
        let src = concat!(
            "public class T {\n",
            "@Test public void testEscapeQuote() {",
            " String s = Utils.dataAsString(new String[] {\"Bobby\\\"s tables\"});",
            " assertTrue(s.contains(\"\\\\\"), s.toString());",
            " }\n}",
        );
        let (blanked, store) = blank_literals(src);

        assert!(blanked.contains("s"), "identifier 's' must survive");
        assert!(blanked.contains("Utils"), "identifier 'Utils' must survive");
        assert!(
            blanked.contains("assertTrue"),
            "identifier 'assertTrue' must survive"
        );
        assert_eq!(
            restore_literals(&blanked, &store),
            src,
            "round-trip must be lossless"
        );
    }

    // ── corruption-recovery heuristic tests ─────────────────────────────────

    #[test]
    fn consume_string_valid_double_backslash_not_split() {
        let src = "assertThat(result).isEqualTo(\"\\\\\\\\\");";
        let (blanked, store) = blank_literals(src);
        assert_eq!(store.entries.len(), 1, "valid \\\\\\\\ must be one literal");
        assert_eq!(
            restore_literals(&blanked, &store),
            src,
            "round-trip must be lossless"
        );
    }

    #[test]
    fn consume_string_corrupt_json_in_java_is_single_literal() {
        let src = "isEqualTo(\"{\\\\\"message\\\\\":\\\\\"val\\\\\"}\");";
        let (blanked, store) = blank_literals(src);
        assert_eq!(
            store.entries.len(),
            1,
            "corrupt JSON string must be one literal"
        );
        assert!(blanked.contains("isEqualTo"), "method call must survive");
        assert_eq!(
            restore_literals(&blanked, &store),
            src,
            "round-trip must be lossless"
        );
    }

    #[test]
    fn consume_string_corrupt_json_array_with_comma_is_single_literal() {
        let src = "equalTo(\"{\\\\\"4\\\\\":[\\\\\"abcd\\\\\",\\\\\"adcb\\\\\"],\\\\\"5\\\\\":[\\\\\"deff\\\\\"]}\")";
        let (blanked, store) = blank_literals(src);
        assert_eq!(
            store.entries.len(),
            1,
            "JSON array string must be one literal"
        );
        assert!(blanked.contains("equalTo"), "method call must survive");
        assert_eq!(
            restore_literals(&blanked, &store),
            src,
            "round-trip must be lossless"
        );
    }

    #[test]
    fn consume_string_corrupt_json_object_with_array_is_single_literal() {
        let src = "readValue(\"{\\\\\"2\\\\\":[\\\\\"ABCDEF\\\\\"],\\\\\"4\\\\\":[\\\\\"FEDCEB\\\\\"]}\", TeamTagMap.class)";
        let (blanked, store) = blank_literals(src);
        assert_eq!(
            store.entries.len(),
            1,
            "JSON string is one literal; TeamTagMap.class has none"
        );
        assert_eq!(
            restore_literals(&blanked, &store),
            src,
            "round-trip must be lossless"
        );
    }

    #[test]
    fn consume_string_apostrophe_after_corrupt_quote_is_suspicious() {
        let src = "p.rot13(\"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ1234567890{}\\\\\"':=\")";
        let (blanked, store) = blank_literals(src);
        assert_eq!(
            store.entries.len(),
            1,
            "string ending in backslash-quote-apostrophe must be one literal, not split at the apostrophe"
        );
        assert!(blanked.contains("p.rot13"), "method call must survive");
        assert_eq!(
            restore_literals(&blanked, &store),
            src,
            "round-trip must be lossless"
        );
    }

    #[test]
    fn blanked_form_is_clean_testclass10222_corrupt_backslash_quote() {
        let src = concat!(
            "public class TestClass10222 {\n",
            "@Test public void should_load_spec() throws Exception {",
            " RestxSpec spec = new RestxSpecLoader(Factory.getInstance()).load(\"cases/test/test.spec.yaml\");",
            " assertThat(spec.getTitle()).isEqualTo(\"should say hello\");",
            " assertThat(spec.getWhens()).extracting(\"then\").extracting(\"expectedCode\", \"expected\")",
            " .containsExactly(Tuple.tuple(200, \"{\\\\\"message\\\\\":\\\\\"hello xavier, it's 14:33:18\\\\\"}\"));",
            " }\n}",
        );
        let (blanked, store) = blank_literals(src);

        assert!(
            blanked.contains("RestxSpec spec"),
            "local 'spec' declaration must be visible"
        );
        assert!(
            blanked.contains("should_load_spec"),
            "method name must be visible"
        );
        assert!(
            !blanked.contains("\\\""),
            "no escaped quotes in blanked source"
        );
        assert_eq!(
            restore_literals(&blanked, &store),
            src,
            "round-trip must be lossless"
        );
    }

    #[test]
    fn consume_string_array_initializer_closing_brace_not_suspicious() {
        let src = "String[] a = new String[]{\"foo\"};";
        let (blanked, store) = blank_literals(src);
        assert_eq!(
            store.entries.len(),
            1,
            "array initializer must produce exactly one literal"
        );
        assert_eq!(
            restore_literals(&blanked, &store),
            src,
            "round-trip must be lossless"
        );
    }

    // ── UTF-8 round-trip tests ───────────────────────────────────────────────

    /// Regression: TestClass11558 — `é` must survive blank_literals→restore_literals
    /// without being mojibake'd into `Ã©`.
    ///
    /// Root cause: `consume_string_literal` previously used
    /// `bytes[..].iter().map(|&b| b as char).collect()` which is a Latin-1
    /// re-interpretation of the bytes. A 2-byte UTF-8 sequence like `é`
    /// (0xC3 0xA9) was decoded as two separate chars `Ã` (U+00C3) and
    /// `©` (U+00A9), causing a token-count mismatch between the prompt
    /// (obfuscated, restored) and the response (original) sides of the JSONL pair.
    #[test]
    fn round_trip_preserves_utf8_accented_chars() {
        let src = "assertThat(var_4, is(\"Bonjour John Doe, le test unitaire est passé\"));";
        let result = round_trip(src);
        assert_eq!(
            result, src,
            "round-trip must preserve UTF-8 multi-byte characters (é must not become Ã©)"
        );
        assert!(
            result.contains('é'),
            "é (U+00E9) must survive blank_literals → restore_literals"
        );
        assert!(
            !result.contains("Ã©"),
            "mojibake 'Ã©' must NOT appear after round-trip"
        );
    }

    #[test]
    fn round_trip_preserves_various_utf8_chars() {
        // A broader set: French, German, Chinese, emoji — all multi-byte UTF-8.
        let src = "assertEquals(\"München Ärger naïve 中文 🎉\", result);";
        let result = round_trip(src);
        assert_eq!(
            result, src,
            "round-trip must preserve all UTF-8 multi-byte characters"
        );
    }

    #[test]
    fn consume_string_literal_stores_utf8_correctly() {
        // Directly test that the stored original literal is UTF-8, not Latin-1.
        let src = "String s = \"passé\";";
        let (_, store) = blank_literals(src);
        assert_eq!(store.entries.len(), 1);
        assert_eq!(
            store.entries[0].original, "\"passé\"",
            "stored literal must be the original UTF-8 string, not mojibake"
        );
    }
}
