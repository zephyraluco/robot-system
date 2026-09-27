// text.rs — Shared text helpers.
//
// Greedy word wrapping used to exist as four near-identical private copies
// (`dialogs/permission.rs`, `dialogs/ask_user_dialog.rs`,
// `dialogs/elicitation_dialog.rs`, `messages/markdown.rs`) plus one dead
// copy in `dialogs/dialog.rs`. They are all this function now: one
// implementation, one set of width/long-token rules, one place to fix.

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Greedy word wrap to `width` display columns.
///
/// * `width == 0`, or text that already fits, is returned as a single line.
/// * Runs of whitespace — newlines included — collapse to single spaces, so
///   callers that must keep hard line breaks split on `'\n'` first.
/// * A single token wider than `width` (a Windows path, URL, base64 blob…) is
///   hard-broken at character boundaries, never inside a grapheme cluster, so
///   it cannot overflow the dialog border.
///
/// Always returns at least one line, so callers can render the result without
/// a special case for empty input.
pub(crate) fn wrap(text: &str, width: usize) -> Vec<String> {
    if width == 0 || UnicodeWidthStr::width(text) <= width {
        return vec![text.to_string()];
    }

    let mut result = Vec::new();
    let mut current_line = String::new();
    let mut current_width = 0usize;

    for word in text.split_whitespace() {
        let word_w = UnicodeWidthStr::width(word);

        // Long unbreakable token: flush the current line, then hard-break the
        // token across as many lines as it needs. The final chunk stays in
        // `current_line` so the next short word can still share that line.
        if word_w > width {
            if !current_line.is_empty() {
                result.push(std::mem::take(&mut current_line));
            }
            let mut chunk = String::new();
            let mut chunk_w = 0usize;
            for ch in word.chars() {
                let cw = UnicodeWidthChar::width(ch).unwrap_or(0);
                if chunk_w + cw > width && !chunk.is_empty() {
                    result.push(std::mem::take(&mut chunk));
                    chunk_w = 0;
                }
                chunk.push(ch);
                chunk_w += cw;
            }
            current_line = chunk;
            current_width = chunk_w;
            continue;
        }

        if current_width == 0 {
            current_line.push_str(word);
            current_width = word_w;
        } else if current_width + 1 + word_w <= width {
            current_line.push(' ');
            current_line.push_str(word);
            current_width += 1 + word_w;
        } else {
            result.push(std::mem::take(&mut current_line));
            current_line.push_str(word);
            current_width = word_w;
        }
    }

    if !current_line.is_empty() {
        result.push(current_line);
    }
    if result.is_empty() {
        // Whitespace-only input: keep the "at least one line" contract.
        result.push(text.to_string());
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_text_unchanged() {
        assert_eq!(wrap("hello world", 40), vec!["hello world"]);
        assert_eq!(wrap("", 10), vec![""]);
        assert_eq!(wrap("hello", 0), vec!["hello"]);
    }

    #[test]
    fn long_text_splits_within_width() {
        let text = "one two three four five six seven eight";
        for line in &wrap(text, 10) {
            assert!(
                UnicodeWidthStr::width(line.as_str()) <= 10,
                "line too wide: {line:?}"
            );
        }
    }

    #[test]
    fn hard_breaks_token_longer_than_width() {
        // A single token wider than the available width must be hard-broken at
        // character boundaries — otherwise it overflows the dialog border (the
        // bug that produced `~X~:~\~B~i~g~g~e~r~…`-style wrapping reports).
        let path = "'X:\\Bigger-Projects\\some-very-long-directory-name'";
        let wrapped = wrap(path, 16);
        assert!(wrapped.len() >= 2, "expected hard-break, got: {wrapped:?}");
        for line in &wrapped {
            assert!(
                UnicodeWidthStr::width(line.as_str()) <= 16,
                "hard-broken chunk too wide: {line:?}"
            );
        }
        // Round-trip: concatenating the chunks rebuilds the token verbatim.
        assert_eq!(wrapped.join(""), path);
    }

    #[test]
    fn mixed_short_and_long_tokens() {
        // The realistic shape that broke the permission dialogs: a normal
        // command followed by a path longer than the column budget.
        let cmd = "git diff 'X:\\Bigger-Projects\\Claurst\\very\\deep\\nested\\path.rs'";
        for line in &wrap(cmd, 24) {
            assert!(
                UnicodeWidthStr::width(line.as_str()) <= 24,
                "line wider than width: {line:?}"
            );
        }
    }

    #[test]
    fn measures_display_width_not_bytes() {
        // Two CJK chars are 6 bytes but only 4 columns: byte-based wrapping
        // used to break this far too early.
        let wrapped = wrap("你好世界", 4);
        assert_eq!(wrapped, vec!["你好", "世界"]);
    }
}
