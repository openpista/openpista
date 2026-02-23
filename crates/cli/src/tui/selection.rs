//! Text selection state and helpers for the TUI chat area.
//!
//! Provides mouse-driven selection tracking, a character grid that mirrors
//! ratatui's `Wrap { trim: false }` layout, selected-text extraction, and
//! clipboard copy via `arboard`.

use ratatui::text::Line;
use unicode_width::UnicodeWidthChar as _;

// ── TextSelection ────────────────────────────────────────────────────────────

/// Tracks mouse-driven text selection inside the chat inner area.
///
/// Coordinates are **screen-relative** `(row, col)` offsets from the top-left
/// corner of the chat widget's inner area (i.e., after the border is removed).
#[derive(Debug, Default)]
pub struct TextSelection {
    /// The point where the drag started.
    pub anchor: Option<(u16, u16)>,
    /// The current or final drag endpoint.
    pub endpoint: Option<(u16, u16)>,
    /// `true` while a left-button drag is still in progress.
    pub dragging: bool,
}

impl TextSelection {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns `true` when a non-degenerate selection exists (anchor ≠ endpoint).
    pub fn is_active(&self) -> bool {
        match (self.anchor, self.endpoint) {
            (Some(a), Some(e)) => a != e,
            _ => false,
        }
    }

    /// Clears anchor, endpoint, and drag state.
    pub fn clear(&mut self) {
        self.anchor = None;
        self.endpoint = None;
        self.dragging = false;
    }

    /// Returns `(start, end)` in reading order
    /// (`start.row ≤ end.row`; if equal, `start.col ≤ end.col`).
    pub fn ordered_range(&self) -> Option<((u16, u16), (u16, u16))> {
        let a = self.anchor?;
        let e = self.endpoint?;
        if a.0 < e.0 || (a.0 == e.0 && a.1 <= e.1) {
            Some((a, e))
        } else {
            Some((e, a))
        }
    }
}

// ── Text grid ────────────────────────────────────────────────────────────────

/// Converts a slice of ratatui [`Line`]s into a 2-D character grid that
/// matches `Wrap { trim: false }` character-level wrapping at `inner_width`
/// display columns.
///
/// Each outer element is one **visual** (wrapped) row; each inner element is
/// the character that occupies that display column.  Double-width characters
/// (CJK, emoji, …) occupy two consecutive columns; the second column stores
/// `'\u{FFFF}'` as a marker so that grid column indices correspond 1-to-1 with
/// terminal cell positions.
pub fn compute_text_grid(lines: &[Line<'_>], inner_width: u16) -> Vec<Vec<char>> {
    let width = inner_width as usize;
    let mut grid: Vec<Vec<char>> = Vec::new();

    for line in lines {
        let mut row: Vec<char> = Vec::new();
        let mut col = 0usize;

        for span in &line.spans {
            for ch in span.content.chars() {
                let ch_w = ch.width().unwrap_or(1).max(1);

                // Wrap before adding this character if it wouldn't fit.
                if width > 0 && col + ch_w > width {
                    while row.len() < width {
                        row.push(' ');
                    }
                    grid.push(row);
                    row = Vec::new();
                    col = 0;
                }

                row.push(ch);
                col += ch_w;

                // Placeholder cell for the second half of a double-width char.
                if ch_w == 2 {
                    row.push('\u{FFFF}');
                }
            }
        }

        // Push the row even if it's empty (blank lines must be preserved).
        grid.push(row);
    }

    grid
}

// ── Text extraction ──────────────────────────────────────────────────────────

/// Extracts the text covered by a selection from `grid`.
///
/// `start` and `end` are **screen-relative** `(row, col)` coordinates inside
/// the chat inner area; `scroll` is `app.history_scroll` so that screen rows
/// can be mapped to grid rows (`grid_row = screen_row + scroll`).
///
/// Returns `None` when the selection is empty or out of range.
pub fn extract_selected_text(
    grid: &[Vec<char>],
    start: (u16, u16),
    end: (u16, u16),
    scroll: u16,
) -> Option<String> {
    if start == end || grid.is_empty() {
        return None;
    }

    let start_grid = (start.0 as usize).saturating_add(scroll as usize);
    let end_grid = (end.0 as usize).saturating_add(scroll as usize);
    let start_col = start.1 as usize;
    let end_col = end.1 as usize;

    let last_row = end_grid.min(grid.len().saturating_sub(1));
    if start_grid > last_row {
        return None;
    }

    let mut result = String::new();

    for (offset, row) in grid
        .iter()
        .enumerate()
        .skip(start_grid)
        .take(last_row - start_grid + 1)
    {
        let grid_row = offset; // offset == start_grid + iteration index

        let (c_start, c_end) = if start_grid == end_grid {
            (start_col, end_col)
        } else if grid_row == start_grid {
            (start_col, row.len())
        } else if grid_row == end_grid {
            (0, end_col)
        } else {
            (0, row.len())
        };

        let c_start = c_start.min(row.len());
        let c_end = c_end.min(row.len());

        if grid_row > start_grid {
            result.push('\n');
        }

        for &ch in &row[c_start..c_end] {
            if ch != '\u{FFFF}' {
                result.push(ch);
            }
        }
    }

    if result.is_empty() {
        None
    } else {
        Some(result)
    }
}

// ── Clipboard ────────────────────────────────────────────────────────────────

/// Copies `text` to the system clipboard.
///
/// Silently swallows errors — clipboard access may not be available in all
/// terminal environments (e.g. headless CI, SSH without X11 forwarding).
pub fn copy_to_clipboard(text: &str) {
    if let Ok(mut cb) = arboard::Clipboard::new() {
        let _ = cb.set_text(text.to_owned());
    }
}

// ── Unit tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::text::{Line, Span};

    // ── compute_text_grid ──

    #[test]
    fn grid_single_short_line() {
        let lines = vec![Line::from("hello")];
        let grid = compute_text_grid(&lines, 20);
        assert_eq!(grid.len(), 1);
        assert_eq!(grid[0], vec!['h', 'e', 'l', 'l', 'o']);
    }

    #[test]
    fn grid_long_line_wraps() {
        // 10-char line into width-5 → 2 rows
        let lines = vec![Line::from("0123456789")];
        let grid = compute_text_grid(&lines, 5);
        assert_eq!(grid.len(), 2);
        // First row fills exactly to width — no padding added.
        assert_eq!(grid[0], vec!['0', '1', '2', '3', '4']);
        assert_eq!(grid[1], vec!['5', '6', '7', '8', '9']);
    }

    #[test]
    fn grid_multi_span() {
        let lines = vec![Line::from(vec![Span::raw("ab"), Span::raw("cd")])];
        let grid = compute_text_grid(&lines, 10);
        assert_eq!(grid.len(), 1);
        assert_eq!(grid[0], vec!['a', 'b', 'c', 'd']);
    }

    #[test]
    fn grid_double_width_char() {
        // '中' is 2 display columns wide
        let lines = vec![Line::from("中a")];
        let grid = compute_text_grid(&lines, 10);
        assert_eq!(grid.len(), 1);
        // '中' occupies col 0-1, 'a' at col 2
        assert_eq!(grid[0][0], '中');
        assert_eq!(grid[0][1], '\u{FFFF}');
        assert_eq!(grid[0][2], 'a');
    }

    #[test]
    fn grid_blank_line_preserved() {
        let lines = vec![Line::from("a"), Line::from(""), Line::from("b")];
        let grid = compute_text_grid(&lines, 10);
        assert_eq!(grid.len(), 3);
        assert!(grid[1].is_empty());
    }

    // ── extract_selected_text ──

    #[test]
    fn extract_single_row() {
        let grid = vec![vec!['h', 'e', 'l', 'l', 'o']];
        // select columns 1..3  → "el"
        let result = extract_selected_text(&grid, (0, 1), (0, 3), 0);
        assert_eq!(result, Some("el".to_string()));
    }

    #[test]
    fn extract_multi_row() {
        let grid = vec![
            vec!['a', 'b', 'c'],
            vec!['d', 'e', 'f'],
            vec!['g', 'h', 'i'],
        ];
        // select from row0,col1  →  row2,col1
        let result = extract_selected_text(&grid, (0, 1), (2, 1), 0);
        assert_eq!(result, Some("bc\ndef\ng".to_string()));
    }

    #[test]
    fn extract_with_scroll_offset() {
        let grid = vec![
            vec!['a', 'b'], // grid row 0  (invisible at scroll=1)
            vec!['c', 'd'], // grid row 1  → screen row 0
        ];
        // screen row 0, col 0..2 with scroll=1  → grid row 1
        let result = extract_selected_text(&grid, (0, 0), (0, 2), 1);
        assert_eq!(result, Some("cd".to_string()));
    }

    #[test]
    fn extract_range_clamped() {
        let grid = vec![vec!['a', 'b', 'c']];
        // end col beyond row length should clamp
        let result = extract_selected_text(&grid, (0, 0), (0, 99), 0);
        assert_eq!(result, Some("abc".to_string()));
    }

    #[test]
    fn extract_empty_selection_returns_none() {
        let grid = vec![vec!['a', 'b']];
        assert!(extract_selected_text(&grid, (0, 1), (0, 1), 0).is_none());
    }

    // ── TextSelection::ordered_range ──

    #[test]
    fn ordered_anchor_before_endpoint() {
        let mut sel = TextSelection::new();
        sel.anchor = Some((2, 3));
        sel.endpoint = Some((4, 1));
        let (start, end) = sel.ordered_range().unwrap();
        assert_eq!(start, (2, 3));
        assert_eq!(end, (4, 1));
    }

    #[test]
    fn ordered_anchor_after_endpoint() {
        let mut sel = TextSelection::new();
        sel.anchor = Some((4, 1));
        sel.endpoint = Some((2, 3));
        let (start, end) = sel.ordered_range().unwrap();
        assert_eq!(start, (2, 3));
        assert_eq!(end, (4, 1));
    }

    #[test]
    fn ordered_same_row_anchor_right_of_endpoint() {
        let mut sel = TextSelection::new();
        sel.anchor = Some((1, 5));
        sel.endpoint = Some((1, 2));
        let (start, end) = sel.ordered_range().unwrap();
        assert_eq!(start, (1, 2));
        assert_eq!(end, (1, 5));
    }

    // ── TextSelection::clear ──

    #[test]
    fn clear_resets_all_fields() {
        let mut sel = TextSelection::new();
        sel.anchor = Some((1, 2));
        sel.endpoint = Some((3, 4));
        sel.dragging = true;
        sel.clear();
        assert!(sel.anchor.is_none());
        assert!(sel.endpoint.is_none());
        assert!(!sel.dragging);
    }

    // ── TextSelection::is_active ──

    #[test]
    fn is_active_true_when_anchor_differs_from_endpoint() {
        let mut sel = TextSelection::new();
        sel.anchor = Some((0, 0));
        sel.endpoint = Some((1, 5));
        assert!(sel.is_active());
    }

    #[test]
    fn is_active_false_when_anchor_equals_endpoint() {
        let mut sel = TextSelection::new();
        sel.anchor = Some((2, 3));
        sel.endpoint = Some((2, 3));
        assert!(!sel.is_active());
    }

    // ── extract_selected_text edge cases ──

    #[test]
    fn extract_returns_none_when_start_grid_beyond_last_row() {
        // Grid has 1 row (index 0). Scroll=10 maps screen row 0 to grid row 10,
        // which is out of range.
        let grid = vec![vec!['a', 'b']];
        assert!(extract_selected_text(&grid, (0, 0), (0, 2), 10).is_none());
    }

    #[test]
    fn extract_returns_none_for_empty_grid() {
        let grid: Vec<Vec<char>> = vec![];
        assert!(extract_selected_text(&grid, (0, 0), (0, 5), 0).is_none());
    }

    #[test]
    fn extract_filters_double_width_placeholder() {
        // '中' occupies 2 columns: ['中', '\u{FFFF}', 'a']
        let grid = vec![vec!['中', '\u{FFFF}', 'a']];
        let result = extract_selected_text(&grid, (0, 0), (0, 3), 0);
        // The \u{FFFF} placeholder should be stripped
        assert_eq!(result, Some("中a".to_string()));
    }

    #[test]
    fn extract_returns_none_for_zero_length_slice() {
        // Selecting from col 3..3 on a 2-char row yields nothing after clamping.
        let grid = vec![vec!['a', 'b']];
        let result = extract_selected_text(&grid, (0, 3), (0, 5), 0);
        // start_col clamped to 2, end_col clamped to 2 → empty slice → None
        assert!(result.is_none());
    }

    // ── compute_text_grid: wrap with padding ──

    #[test]
    fn grid_wrap_pads_short_row_before_new_row() {
        // Width = 4. A double-width char ('中', w=2) at col 3 doesn't fit,
        // so the current row (3 chars) is padded to width 4 before wrapping.
        let lines = vec![Line::from("abc中")];
        let grid = compute_text_grid(&lines, 4);
        // Row 0: ['a','b','c',' '] (padded to width 4 because '中' didn't fit)
        // Row 1: ['中','\u{FFFF}']
        assert_eq!(grid.len(), 2);
        assert_eq!(grid[0].len(), 4);
        assert_eq!(grid[0][3], ' '); // padding space
        assert_eq!(grid[1][0], '中');
    }
}
