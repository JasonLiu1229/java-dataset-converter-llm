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
/// Replacements are applied left-to-right, matching the insertion order, so
/// the function is correct even when multiple literals share the same content.
pub fn restore_literals(blanked: &str, store: &LiteralStore) -> String {
    if store.entries.is_empty() {
        return blanked.to_string();
    }

    let mut result = blanked.to_string();

    for entry in &store.entries {
        if let Some(pos) = result.find(&entry.placeholder) {
            result.replace_range(pos..pos + entry.placeholder.len(), &entry.original);
        }
        // A missing placeholder means the processing step deleted the literal,
        // which is a caller bug.  Skip silently so the rest is still restored.
    }

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
/// (a letter, digit, `{`, `}`, `:`, `_`, `$`, or `\`), the close is almost
/// certainly spurious corruption and we treat the `"` as an embedded escaped
/// quote instead.
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
                        // Even run: all backslashes form escape pairs, so the `"` would
                        // normally be the closing quote.  Apply the corruption heuristic.
                        let next = bytes.get(i + 1).copied().unwrap_or(b' ');
                        if is_suspicious_after_even_backslash_close(next) {
                            // Looks like a corrupt `\\"` (should be `\"`).  Treat as
                            // embedded and keep scanning.
                            i += 1;
                        } else {
                            // The `"` is genuinely the closing quote.
                            i += 1;
                            break;
                        }
                    } else {
                        // Odd run: the last backslash escapes the `"` — embedded quote,
                        // keep scanning.
                        i += 1;
                    }
                }
                // If the run is not followed by `"`, the bytes were already consumed;
                // just continue the outer loop.
            }
            b'"' => {
                // Bare unescaped closing quote — always ends the string.
                i += 1;
                break;
            }
            _ => {
                i += 1;
            }
        }
    }

    (String::from_utf8_lossy(&bytes[start..i]).into_owned(), i)
}

/// Returns `Some((literal_text, one_past_end))` for a well-formed char
/// literal, `None` for anything that doesn't look like one (stray apostrophe).
fn try_consume_char_literal(bytes: &[u8], start: usize) -> Option<(String, usize)> {
    debug_assert_eq!(bytes[start], b'\'');
    let len = bytes.len();

    if start + 2 >= len {
        return None;
    }

    let mut i = start + 1;

    match bytes[i] {
        b'\\' => {
            i += 1;
            if i >= len {
                return None;
            }
            if bytes[i] == b'u' || bytes[i] == b'U' {
                // Unicode escape  \uXXXX
                i += 1;
                let hex_start = i;
                while i < len && (i - hex_start) < 4 && is_hex(bytes[i]) {
                    i += 1;
                }
                if i - hex_start != 4 {
                    return None;
                }
            } else {
                // Single-char escape  \n  \t  \\  \'  etc.
                i += 1;
            }
        }
        b'\'' | b'\n' | b'\r' => return None, // empty literal or bare newline
        _ => {
            // Plain or multi-byte UTF-8 character.
            let ch = std::str::from_utf8(&bytes[i..]).ok()?.chars().next()?;
            i += ch.len_utf8();
        }
    }

    if i >= len || bytes[i] != b'\'' {
        return None;
    }
    i += 1;

    Some((String::from_utf8_lossy(&bytes[start..i]).into_owned(), i))
}

fn is_hex(b: u8) -> bool {
    matches!(b, b'0'..=b'9' | b'a'..=b'f' | b'A'..=b'F')
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

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

    // ── regression tests from skipped_20260317_155910.log ───────────────────

    /// TestClass10222 — `it's` apostrophe inside a JSON-in-Java string.
    /// The outer string contains `{\"message\":\"hello xavier, it's 14:33:18\"}`
    /// which has both `\"` escape sequences AND a bare apostrophe.
    /// After blanking the identifier `spec` must still be visible; the apostrophe
    /// must be gone (swallowed into the placeholder).
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

    /// TestClass10150 — string literal ending with `{}\"':=`.
    /// The `'` immediately follows `\"` inside the string; the blanker must not
    /// treat that apostrophe as the start of a char literal.
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

    /// TestClass10693 — string with multiple embedded `\"` sequences separated by `.`
    /// `"root.\"sg\".\"d1\".\"s1\"\""` — the final `\"\"` is two adjacent escape
    /// sequences; consume_string_literal must not close at the first one.
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

    /// TestClass20926 — nested escaped quotes `\\\"0.5\\\"` inside a string.
    /// The pattern `\\\"` (two backslashes + quote in the Rust source = one backslash
    /// + quote in the Java file) must be treated as an escape pair by
    /// consume_string_literal; it must NOT be treated as a closing quote.
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

    /// TestClass74779 — the entire string value is a backslash-quote `\"`.
    /// Java source: `"\\\""` — this is `\\` (one literal backslash) + `\"` (escaped
    /// quote), so the string value is `\"`.  consume_string_literal must consume
    /// the whole thing as one literal and not close at the inner `\"`.
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
        // The char literal '"' must be blanked to 'X'
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

    /// TestClass8595 / TestClass8926 / TestClass9090 — entity tag strings of the
    /// form `"\"test \\\"test\\\"\""`.  These contain `\\\"` (escaped backslash
    /// followed by escaped quote) which is two separate escape pairs.
    /// consume_string_literal must handle each pair independently.
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

    /// TestClass67764 — first argument is a single-double-quote string `"\""`.
    /// This is a common pattern in list-splitter tests.  The string value is
    /// just one `"` character; the literal is `"\""`.
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
        // "\"" + "Prague, Boston, \"Helsinki, Finland\"" + "Prague" = 3 literals
        assert_eq!(store.entries.len(), 3, "exactly three string literals");
        assert_eq!(
            restore_literals(&blanked, &store),
            src,
            "round-trip must be lossless"
        );
    }

    /// TestClass78373 — contains a genuinely unterminated-looking string `"\""`
    /// followed by identifiers, then another string.  The blanker must not
    /// accidentally consume method tokens as part of a string.
    #[test]
    fn blanked_does_not_consume_identifiers_into_string() {
        // Simplified version of the TestClass78373 pattern
        let src = concat!(
            "public class T {\n",
            "@Test public void testEscapeQuote() {",
            " String s = Utils.dataAsString(new String[] {\"Bobby\\\"s tables\"});",
            " assertTrue(s.contains(\"\\\\\"), s.toString());",
            " }\n}",
        );
        let (blanked, store) = blank_literals(src);

        // All identifiers must still be outside string placeholders
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

    /// TestClass12696 regression: `"\\\\"` is a valid Java string whose value is
    /// two backslashes.  The even backslash run (4 backslashes) precedes the
    /// closing `"`, and the next char is `)` — not suspicious — so the string
    /// closes correctly and is preserved as a single literal.
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

    /// Corrupt `{\\\"key\\\":\\\"val\\\"}` — dataset encoding artifact where each
    /// `\"` inside a JSON string was doubled to `\\"`.  The heuristic detects that
    /// letters and `:` after the candidate close are suspicious and keeps scanning,
    /// producing one correct literal instead of several broken fragments.
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

    /// TestClass10222 regression: full method body with `\\"` corruption AND
    /// a plain apostrophe inside the string value.  The blanked source must have
    /// all identifiers visible so tree-sitter can parse the method.
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

        // All identifiers must be outside string placeholders.
        assert!(
            blanked.contains("RestxSpec spec"),
            "local 'spec' declaration must be visible"
        );
        assert!(
            blanked.contains("should_load_spec"),
            "method name must be visible"
        );

        // No escaped quotes must remain in the blanked source.
        assert!(
            !blanked.contains("\\\""),
            "no escaped quotes in blanked source"
        );

        // Round-trip must be lossless.
        assert_eq!(
            restore_literals(&blanked, &store),
            src,
            "round-trip must be lossless"
        );
    }

    /// `new String[]{"foo"}` — `}` directly after a plain unescaped `"` is valid
    /// Java (array initializer close).  The heuristic must NOT fire here because
    /// the closing `"` is bare (no preceding backslash run), so it is always treated
    /// as a string terminator regardless of the following character.
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
}
