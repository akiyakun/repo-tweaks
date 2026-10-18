//! Terminal layout helpers shared by the commands.

use unicode_width::UnicodeWidthStr;

/// Pad to a display width (CJK characters take two columns).
pub fn pad(text: &str, width: usize) -> String {
    format!("{text}{}", " ".repeat(width.saturating_sub(text.width())))
}
