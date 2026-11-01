//! External Basecamp boundary; authentication belongs to the installed CLI.
use crate::model::BasecampLink;

pub trait Basecamp {
    fn trash_card(&self, link: &BasecampLink, id: &str) -> Result<(), String>;
    fn find_card(&self, link: &BasecampLink, title: &str) -> Result<Option<String>, String>;
    fn create(&self, link: &BasecampLink, column: &str, title: &str) -> Result<String, String>;
    fn move_card(&self, link: &BasecampLink, id: &str, column: &str) -> Result<(), String>;
    fn update_title(&self, link: &BasecampLink, id: &str, title: &str) -> Result<(), String>;
}

mod real;
pub use real::RealBasecamp;

// Sanitize at the adapter and ops boundaries, including errors from injected adapters.
pub(crate) fn diagnostic(input: &str) -> String {
    let lower = input.to_ascii_lowercase();
    let mut out = String::new();
    let mut at = 0;
    while at < input.len() {
        let tail = &lower[at..];
        let boundary = at == 0 || !input.as_bytes()[at - 1].is_ascii_alphanumeric();
        let mut matched = None;
        if boundary {
            for key in [
                "authorization",
                "access_token",
                "api_key",
                "api-key",
                "token",
                "bearer",
            ] {
                if let Some(rest) = tail.strip_prefix(key) {
                    let quoted = at > 0 && matches!(input.as_bytes()[at - 1], b'"' | b'\'');
                    let rest = if quoted {
                        let quote = input.as_bytes()[at - 1] as char;
                        let Some(rest) = rest.strip_prefix(quote) else {
                            continue;
                        };
                        rest
                    } else {
                        rest
                    };
                    let spaces = rest.len() - rest.trim_start().len();
                    let separator = rest.as_bytes().get(spaces);
                    if (key == "bearer" && spaces > 0)
                        || (key != "bearer" && matches!(separator, Some(b':' | b'=')))
                    {
                        matched = Some((
                            key,
                            at + key.len()
                                + usize::from(quoted)
                                + spaces
                                + usize::from(key != "bearer"),
                        ));
                        break;
                    }
                }
            }
        }
        if let Some((key, mut start)) = matched {
            start += input[start..].len() - input[start..].trim_start().len();
            out.push_str(&input[at..start]);
            // A JSON string marker stays intact when the next boundary sanitizes again.
            out.push_str("\"[REDACTED]\"");
            let rest = &input[start..];
            let (scheme, value) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
            let scheme_bytes = if key == "authorization"
                && ["bearer", "basic"]
                    .iter()
                    .any(|s| scheme.eq_ignore_ascii_case(s))
            {
                rest.len() - value.trim_start().len()
            } else {
                0
            };
            let rest = &rest[scheme_bytes..];
            let end = if rest.starts_with('"') {
                // Let JSON find the real closing quote, including escapes and Unicode.
                let mut strings = serde_json::Deserializer::from_str(rest).into_iter::<String>();
                match strings.next() {
                    Some(Ok(_)) => strings.byte_offset(),
                    // A malformed or unterminated credential must not expose its tail.
                    _ => rest.len(),
                }
            } else if let Some(quote @ '\'') = rest.chars().next() {
                rest[1..].find(quote).map_or(rest.len(), |i| i + 2)
            } else {
                rest.find(|c: char| c.is_whitespace() || matches!(c, '&' | ',' | ';' | '}' | ']'))
                    .unwrap_or(rest.len())
            };
            at = start + scheme_bytes + end;
        } else {
            let ch = input[at..].chars().next().unwrap_or_default();
            out.push(ch);
            at += ch.len_utf8();
        }
    }
    let mut end = out.len().min(2048);
    while !out.is_char_boundary(end) {
        end -= 1;
    }
    out.truncate(end);
    out
}
