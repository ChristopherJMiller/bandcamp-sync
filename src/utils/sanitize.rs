//! Filename sanitization utilities

/// Sanitize a filename for safe filesystem usage
/// 
/// Replaces filesystem-unsafe characters with visually similar Unicode alternatives
/// that are safe to use in filenames across all major operating systems.
/// 
/// # Examples
/// 
/// ```
/// use bandcamp_sync::utils::sanitize_filename;
/// 
/// assert_eq!(sanitize_filename("BOTHERED / UNBOTHERED"), "BOTHERED ⧸ UNBOTHERED");
/// assert_eq!(sanitize_filename("Transistor: Original Soundtrack"), "Transistor꞉ Original Soundtrack");
/// ```
pub fn sanitize_filename(name: &str) -> String {
    // Replace problematic characters with visually similar Unicode alternatives
    name.chars()
        .map(|c| match c {
            '/' => '⧸',  // U+29F8 - Big Solidus (looks like / but is filesystem-safe)
            '\\' => '⧹', // U+29F9 - Big Reverse Solidus  
            ':' => '꞉',  // U+A789 - Modifier Letter Colon
            '*' => '⁎',  // U+204E - Low Asterisk
            '?' => '？', // U+FF1F - Fullwidth Question Mark
            '"' => '″',  // U+2033 - Double Prime
            '<' => '‹',  // U+2039 - Single Left Angle Quote
            '>' => '›',  // U+203A - Single Right Angle Quote
            '|' => '｜', // U+FF5C - Fullwidth Vertical Line
            '\0' => '_', // Null byte has no good lookalike, use underscore
            _ => c,
        })
        .collect::<String>()
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_slashes() {
        assert_eq!(sanitize_filename("BOTHERED / UNBOTHERED"), "BOTHERED ⧸ UNBOTHERED");
        assert_eq!(sanitize_filename("R/Edgelord"), "R⧸Edgelord");
    }

    #[test]
    fn test_sanitize_colon() {
        assert_eq!(sanitize_filename("Transistor: Original Soundtrack"), "Transistor꞉ Original Soundtrack");
    }

    #[test]
    fn test_sanitize_quotes() {
        assert_eq!(sanitize_filename("\"Emerson\" Unreleased Demo"), "″Emerson″ Unreleased Demo");
    }

    #[test]
    fn test_sanitize_triple_slash() {
        assert_eq!(sanitize_filename("LOVE /// DISCONNECT"), "LOVE ⧸⧸⧸ DISCONNECT");
    }

    #[test]
    fn test_no_changes_needed() {
        assert_eq!(sanitize_filename("Normal Album Name"), "Normal Album Name");
    }

    #[test]
    fn test_trim_whitespace() {
        assert_eq!(sanitize_filename("  Album Name  "), "Album Name");
    }
}