//! A user's free time on a board is a set of pieces. Pieces are never
//! rewritten when a range grows, so each keeps the moment it was created.

/// One stretch of a user's free time, `[start, end)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Piece {
    pub id: String,
    pub user_id: String,
    pub start: i64,
    pub end: i64,
    pub created_at: i64,
}

/// What removing a range does to a user's pieces: pieces to delete, and the
/// pieces left over from cut ones as `(start, end, created_at)`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Trim {
    pub delete: Vec<String>,
    pub insert: Vec<(i64, i64, i64)>,
}

/// The user's pieces in a fixed order, so results never depend on input order.
fn ordered(mine: &[Piece]) -> Vec<&Piece> {
    let mut pieces: Vec<&Piece> = mine.iter().filter(|p| p.start < p.end).collect();
    pieces.sort_by(|a, b| (a.start, a.end, &a.id).cmp(&(b.start, b.end, &b.id)));
    pieces
}

/// Touching or overlapping pieces drawn as one, in time order.
pub fn merged(mine: &[Piece]) -> Vec<(i64, i64)> {
    let mut out: Vec<(i64, i64)> = Vec::new();
    for p in ordered(mine) {
        match out.last_mut() {
            Some(last) if p.start <= last.1 => last.1 = last.1.max(p.end),
            _ => out.push((p.start, p.end)),
        }
    }
    out
}

/// The parts of `[start, end)` that none of the user's pieces cover. Each
/// becomes a new piece created now; that is how a range grows.
pub fn uncovered_parts(mine: &[Piece], start: i64, end: i64) -> Vec<(i64, i64)> {
    let mut out = Vec::new();
    let mut cursor = start;
    for (s, e) in merged(mine) {
        if cursor >= end {
            break;
        }
        if e <= cursor {
            continue;
        }
        if s > cursor {
            out.push((cursor, s.min(end)));
        }
        cursor = cursor.max(e);
    }
    if cursor < end {
        out.push((cursor, end));
    }
    out
}

/// Removes `[start, end)` from the user's pieces. A cut piece keeps its
/// `created_at`, and a cut in the middle makes two pieces.
pub fn trim(mine: &[Piece], start: i64, end: i64) -> Trim {
    let mut out = Trim::default();
    for p in ordered(mine) {
        if p.end <= start || p.start >= end {
            continue;
        }
        out.delete.push(p.id.clone());
        if p.start < start {
            out.insert.push((p.start, start, p.created_at));
        }
        if p.end > end {
            out.insert.push((end, p.end, p.created_at));
        }
    }
    out
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use super::*;

    const H: i64 = 3600;

    fn piece(id: &str, start: i64, end: i64, created_at: i64) -> Piece {
        Piece {
            id: id.to_owned(),
            user_id: "ana".to_owned(),
            start,
            end,
            created_at,
        }
    }

    #[test]
    fn extended_range_keeps_original_pieces_time() {
        let mut mine = vec![piece("p1", 10 * H, 12 * H, 100)];
        let parts = uncovered_parts(&mine, 9 * H, 13 * H);
        assert_eq!(parts, vec![(9 * H, 10 * H), (12 * H, 13 * H)]);
        // The caller turns each part into a new piece created now.
        for (i, (s, e)) in parts.into_iter().enumerate() {
            mine.push(piece(&format!("n{i}"), s, e, 200));
        }
        assert_eq!(mine[0].created_at, 100);
        assert_eq!(merged(&mine), vec![(9 * H, 13 * H)]);
    }

    #[test]
    fn fully_covered_range_has_no_new_parts() {
        let mine = vec![piece("p1", 10 * H, 12 * H, 100)];
        assert!(uncovered_parts(&mine, 10 * H, 11 * H).is_empty());
        assert!(uncovered_parts(&mine, 11 * H, 11 * H).is_empty());
    }

    #[test]
    fn merged_joins_touching_and_overlapping_pieces() {
        let mine = vec![
            piece("c", 14 * H, 15 * H, 1),
            piece("a", 10 * H, 11 * H, 1),
            piece("b", 11 * H, 12 * H, 2),
            piece("d", 11 * H + 1800, 12 * H + 1800, 3),
        ];
        assert_eq!(
            merged(&mine),
            vec![(10 * H, 12 * H + 1800), (14 * H, 15 * H)]
        );
    }

    #[test]
    fn trim_keeps_piece_time() {
        let mine = vec![piece("p1", 10 * H, 12 * H, 100)];
        let t = trim(&mine, 11 * H, 12 * H);
        assert_eq!(t.delete, vec!["p1".to_owned()]);
        assert_eq!(t.insert, vec![(10 * H, 11 * H, 100)]);
    }

    #[test]
    fn trim_in_the_middle_splits() {
        let mine = vec![piece("p1", 10 * H, 13 * H, 100)];
        let t = trim(&mine, 11 * H, 12 * H);
        assert_eq!(t.delete, vec!["p1".to_owned()]);
        assert_eq!(t.insert, vec![(10 * H, 11 * H, 100), (12 * H, 13 * H, 100)]);
    }

    #[test]
    fn trim_outside_changes_nothing() {
        let mine = vec![piece("p1", 10 * H, 12 * H, 100)];
        for (s, e) in [(8 * H, 9 * H), (12 * H, 13 * H), (9 * H, 10 * H)] {
            let t = trim(&mine, s, e);
            assert!(t.delete.is_empty() && t.insert.is_empty());
        }
    }

    #[test]
    fn trim_covering_a_piece_deletes_it_whole() {
        let mine = vec![
            piece("p1", 10 * H, 11 * H, 100),
            piece("p2", 11 * H, 12 * H, 5),
        ];
        let t = trim(&mine, 9 * H, 11 * H + 1800);
        assert_eq!(t.delete, vec!["p1".to_owned(), "p2".to_owned()]);
        assert_eq!(t.insert, vec![(11 * H + 1800, 12 * H, 5)]);
    }
}
