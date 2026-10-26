//! Where events go: side by side when they overlap in the time grid, and in
//! lanes when they span days.

/// An event's place among the events it overlaps: column `col` of `cols`,
/// widened over `span` columns when the ones to its right are free, and
/// indented by `depth` steps over the earlier events it's drawn on top of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placement {
    pub col: usize,
    pub span: usize,
    pub cols: usize,
    pub depth: usize,
}

/// Layout for one day of the time grid, like Google Calendar's. Events
/// that start within `gap` of each other would hide each other's titles,
/// so they sit side by side in columns. An event that starts later than
/// that cascades: it's drawn on top of the earlier ones, indented, keeping
/// most of the width, while their titles stay visible above it.
///
/// `spans` are `(start, end)` in any unit, end exclusive; the result is in
/// the same order. Events that overlap, directly or through others, form a
/// cluster that shares one column count.
pub fn pack_columns(spans: &[(i64, i64)], gap: i64) -> Vec<Placement> {
    let mut order: Vec<usize> = (0..spans.len()).collect();
    order.sort_by_key(|&i| (spans[i].0, std::cmp::Reverse(spans[i].1)));

    let overlaps = |a: usize, b: usize| spans[a].0 < spans[b].1 && spans[b].0 < spans[a].1;
    let conflicts = |a: usize, b: usize| overlaps(a, b) && (spans[a].0 - spans[b].0).abs() < gap;

    let mut placements = vec![
        Placement {
            col: 0,
            span: 1,
            cols: 1,
            depth: 0,
        };
        spans.len()
    ];
    let mut cluster: Vec<usize> = Vec::new();
    let mut cluster_end = i64::MIN;

    for i in order {
        let (start, end) = spans[i];
        if start >= cluster_end && !cluster.is_empty() {
            finish(&mut placements, &mut cluster, &conflicts, &overlaps);
        }
        // The first column without an event this one conflicts with.
        let col = (0..)
            .find(|&c| {
                !cluster
                    .iter()
                    .any(|&j| placements[j].col == c && conflicts(i, j))
            })
            .expect("some column is free");
        placements[i].col = col;
        cluster.push(i);
        cluster_end = if cluster.len() == 1 {
            end
        } else {
            cluster_end.max(end)
        };
    }
    if !cluster.is_empty() {
        finish(&mut placements, &mut cluster, &conflicts, &overlaps);
    }
    placements
}

/// Give a finished cluster (in start order) its column count, widen each
/// event over the columns to its right that hold nothing it conflicts
/// with, and count the earlier events it's drawn over.
fn finish(
    placements: &mut [Placement],
    cluster: &mut Vec<usize>,
    conflicts: &impl Fn(usize, usize) -> bool,
    overlaps: &impl Fn(usize, usize) -> bool,
) {
    let cols = cluster
        .iter()
        .map(|&i| placements[i].col + 1)
        .max()
        .unwrap_or(1);
    for &i in cluster.iter() {
        placements[i].cols = cols;
        let col = placements[i].col;
        let blocked = |c: usize| {
            cluster
                .iter()
                .any(|&j| placements[j].col == c && conflicts(i, j))
        };
        placements[i].span = 1 + (col + 1..cols).take_while(|&c| !blocked(c)).count();
    }
    for (n, &i) in cluster.iter().enumerate() {
        let (col, span) = (placements[i].col, placements[i].span);
        placements[i].depth = cluster[..n]
            .iter()
            .filter(|&&j| {
                let p = placements[j];
                overlaps(i, j) && !conflicts(i, j) && p.col < col + span && col < p.col + p.span
            })
            .count();
    }
    cluster.clear();
}

/// Lanes for bars across days (all-day and multi-day events): each takes the
/// first lane free over its days. `spans` are `(first_day, end_day)`, end
/// exclusive; the result is each one's lane, in the same order.
pub fn pack_lanes(spans: &[(i64, i64)]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..spans.len()).collect();
    order.sort_by_key(|&i| (spans[i].0, std::cmp::Reverse(spans[i].1 - spans[i].0)));
    let mut lane_ends: Vec<i64> = Vec::new();
    let mut lanes = vec![0; spans.len()];
    for i in order {
        let (start, end) = spans[i];
        lanes[i] = match lane_ends.iter().position(|&e| e <= start) {
            Some(lane) => {
                lane_ends[lane] = end;
                lane
            }
            None => {
                lane_ends.push(end);
                lane_ends.len() - 1
            }
        };
    }
    lanes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(col: usize, span: usize, cols: usize, depth: usize) -> Placement {
        Placement {
            col,
            span,
            cols,
            depth,
        }
    }

    const GAP: i64 = 30;

    #[test]
    fn separate_events_take_the_full_width() {
        assert_eq!(
            pack_columns(&[(0, 60), (60, 120), (200, 230)], GAP),
            vec![p(0, 1, 1, 0), p(0, 1, 1, 0), p(0, 1, 1, 0)]
        );
    }

    #[test]
    fn events_starting_together_sit_side_by_side() {
        assert_eq!(
            pack_columns(&[(0, 60), (15, 90)], GAP),
            vec![p(0, 1, 2, 0), p(1, 1, 2, 0)]
        );
        // Three at once.
        assert_eq!(
            pack_columns(&[(0, 60), (0, 60), (0, 60)], GAP),
            vec![p(0, 1, 3, 0), p(1, 1, 3, 0), p(2, 1, 3, 0)]
        );
    }

    #[test]
    fn later_events_cascade_over_earlier_ones() {
        // B starts well after A: full width, one step in, over A.
        assert_eq!(
            pack_columns(&[(0, 120), (30, 90)], GAP),
            vec![p(0, 1, 1, 0), p(0, 1, 1, 1)]
        );
        // A chain: each over all the ones before it.
        assert_eq!(
            pack_columns(&[(0, 180), (30, 150), (60, 120)], GAP),
            vec![p(0, 1, 1, 0), p(0, 1, 1, 1), p(0, 1, 1, 2)]
        );
    }

    #[test]
    fn cascades_cover_side_by_side_events() {
        // Three at 9, and one at 9:30 over all three, at full width.
        assert_eq!(
            pack_columns(&[(0, 60), (0, 60), (0, 60), (30, 60)], GAP),
            vec![p(0, 1, 3, 0), p(1, 1, 3, 0), p(2, 1, 3, 0), p(0, 3, 3, 3)]
        );
    }

    #[test]
    fn events_widen_into_free_columns() {
        // A and B start together; C, after A, widens over B's column.
        assert_eq!(
            pack_columns(&[(0, 200), (0, 50), (60, 100)], GAP),
            vec![p(0, 1, 2, 0), p(1, 1, 2, 0), p(0, 2, 2, 1)]
        );
    }

    #[test]
    fn input_order_does_not_matter() {
        assert_eq!(
            pack_columns(&[(10, 90), (0, 60)], GAP),
            vec![p(1, 1, 2, 0), p(0, 1, 2, 0)]
        );
    }

    #[test]
    fn lanes() {
        // Mon-Wed, Tue, Thu-Fri, Mon.
        assert_eq!(
            pack_lanes(&[(0, 3), (1, 2), (3, 5), (0, 1)]),
            vec![0, 1, 0, 1]
        );
        assert_eq!(pack_lanes(&[]), Vec::<usize>::new());
    }
}
