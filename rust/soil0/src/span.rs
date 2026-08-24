//! Source spans, contract §3: UTF-8 byte offsets are canonical
//! (`start`/`end`, end-exclusive); `line`/`col` are 1-based, derived from
//! `start`, display-only. Columns count bytes from the line start.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub line: usize,
    pub col: usize,
}

/// The contract §4 node wrapper: `{"span": …, "item": …}` — every tree
/// node in CLI output is wrapped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Spanned<T> {
    pub span: Span,
    pub item: T,
}

/// Derives `line`/`col` from byte offsets for one source text.
pub struct LineMap {
    /// Byte offset at which each line starts; `line_starts[0] == 0`.
    line_starts: Vec<usize>,
}

impl LineMap {
    pub fn new(src: &str) -> Self {
        let mut line_starts = vec![0];
        for (i, b) in src.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(i + 1);
            }
        }
        LineMap { line_starts }
    }

    pub fn span(&self, start: usize, end: usize) -> Span {
        debug_assert!(start <= end);
        let line_idx = match self.line_starts.binary_search(&start) {
            Ok(i) => i,
            Err(i) => i - 1,
        };
        Span {
            start,
            end,
            line: line_idx + 1,
            col: start - self.line_starts[line_idx] + 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_col_derivation() {
        let map = LineMap::new("ab\ncd\n\nx");
        assert_eq!(
            map.span(0, 2),
            Span {
                start: 0,
                end: 2,
                line: 1,
                col: 1
            }
        );
        assert_eq!(
            map.span(1, 2),
            Span {
                start: 1,
                end: 2,
                line: 1,
                col: 2
            }
        );
        assert_eq!(
            map.span(3, 5),
            Span {
                start: 3,
                end: 5,
                line: 2,
                col: 1
            }
        );
        assert_eq!(
            map.span(6, 6),
            Span {
                start: 6,
                end: 6,
                line: 3,
                col: 1
            }
        );
        assert_eq!(
            map.span(7, 8),
            Span {
                start: 7,
                end: 8,
                line: 4,
                col: 1
            }
        );
    }

    #[test]
    fn span_json_field_order() {
        let s = Span {
            start: 42,
            end: 47,
            line: 3,
            col: 5,
        };
        assert_eq!(
            serde_json::to_string(&s).unwrap(),
            r#"{"start":42,"end":47,"line":3,"col":5}"#
        );
    }
}
