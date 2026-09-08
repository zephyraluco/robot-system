// input.rs — Slash command helpers and input mode types.

use crossterm::event::KeyModifiers;

/// Apply the US-QWERTY shift map to a printable character reported as
/// unshifted + SHIFT (kitty keyboard protocol terminals).
///
/// **Keyboard layout limitation**: This only works correctly for US QWERTY keyboards.
/// Other layouts (AZERTY, QWERTZ, etc.) have different shift mappings. For non-US
/// layouts, we rely on the terminal to send the correctly shifted character, which
/// most modern terminals do (especially with kitty protocol enabled).
pub fn normalize_char_with_shift(c: char, modifiers: KeyModifiers) -> char {
    if !modifiers.contains(KeyModifiers::SHIFT) {
        return c;
    }

    if c.is_ascii_lowercase() {
        return c.to_ascii_uppercase();
    }

    // Map unshifted number/symbol keys to their shifted equivalents (US QWERTY)
    match c {
        '1' => '!',
        '2' => '@',
        '3' => '#',
        '4' => '$',
        '5' => '%',
        '6' => '^',
        '7' => '&',
        '8' => '*',
        '9' => '(',
        '0' => ')',
        '-' => '_',
        '=' => '+',
        '[' => '{',
        ']' => '}',
        ';' => ':',
        '\'' => '"',
        ',' => '<',
        '.' => '>',
        '/' => '?',
        '\\' => '|',
        '`' => '~',
        _ => c,
    }
}

/// Check whether a string looks like a slash command (e.g. "/help").
pub fn is_slash_command(input: &str) -> bool {
    input.starts_with('/') && !input.starts_with("//")
}

/// Parse a slash command into `(command_name, args)`.
/// Returns `("", "")` if the input is not a slash command.
pub fn parse_slash_command(input: &str) -> (&str, &str) {
    if !is_slash_command(input) {
        return ("", "");
    }
    let without_slash = &input[1..];
    if let Some(space_idx) = without_slash.find(' ') {
        (
            &without_slash[..space_idx],
            without_slash[space_idx + 1..].trim(),
        )
    } else {
        (without_slash, "")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slash_command_detection() {
        assert!(is_slash_command("/help"));
        assert!(is_slash_command("/compact args"));
        assert!(!is_slash_command("//comment"));
        assert!(!is_slash_command("hello"));
        assert!(!is_slash_command(""));
    }

    #[test]
    fn parse_no_args() {
        let (cmd, args) = parse_slash_command("/help");
        assert_eq!(cmd, "help");
        assert_eq!(args, "");
    }

    #[test]
    fn parse_with_args() {
        let (cmd, args) = parse_slash_command("/compact  --force ");
        assert_eq!(cmd, "compact");
        assert_eq!(args, "--force");
    }

    #[test]
    fn parse_non_slash() {
        let (cmd, args) = parse_slash_command("hello world");
        assert_eq!(cmd, "");
        assert_eq!(args, "");
    }
}
