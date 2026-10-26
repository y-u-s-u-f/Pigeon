//! Where events go: side by side when they overlap in the time grid, and in
//! lanes when they span days.

/// An event's place among the events it overlaps: column `col` of `cols`,
/// widened over `span` columns when the ones to its right are free.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placement {
    pub col: usize,
    pub span: usize,
    pub cols: usize,
}

/// Column packing for one day of the time grid, like Google Calendar's:
/// events that overlap, directly or through others, form a cluster that
/// shares the day's width; each takes the first column free at its start.
/// `spans` are `(start, end)` in any unit, end exclusive; the result is in
/// the same order.
pub fn pack_columns(spans: &[(i64, i64)]) -> Vec<Placement> {
    let mut order: Vec<usize> = (0..spans.len()).collect();
    order.sort_by_key(|&i| (spans[i].0, std::cmp::Reverse(spans[i].1)));

    let mut placements = vec![
        Placement {
            col: 0,
            span: 1,
            cols: 1,
        };
        spans.len()
    ];
    let mut cluster: Vec<usize> = Vec::new();
    // The end of the last event in each column of the cluster.
    let mut column_ends: Vec<i64> = Vec::new();
    let mut cluster_end = i64::MIN;

    for i in order {
        let (start, end) = spans[i];
        if start >= cluster_end && !cluster.is_empty() {
            finish(spans, &mut placements, &mut cluster, &mut column_ends);
        }
        let col = match column_ends.iter().position(|&e| e <= start) {
            Some(col) => {
                column_ends[col] = end;
                col
            }
            None => {
                column_ends.push(end);
                column_ends.len() - 1
            }
        };
        placements[i].col = col;
        cluster.push(i);
        cluster_end = if cluster.len() == 1 {
            end
        } else {
            cluster_end.max(end)
        };
    }
    if !cluster.is_empty() {
        finish(spans, &mut placements, &mut cluster, &mut column_ends);
    }
    placements
}

/// Give a finished cluster its column count, and widen each of its events
/// over the free columns to its right.
fn finish(
    spans: &[(i64, i64)],
    placements: &mut [Placement],
    cluster: &mut Vec<usize>,
    column_ends: &mut Vec<i64>,
) {
    let cols = column_ends.len();
    for &i in cluster.iter() {
        placements[i].cols = cols;
    }
    for &i in cluster.iter() {
        let (start, end) = spans[i];
        let col = placements[i].col;
        let blocked = |c: usize| {
            cluster
                .iter()
                .any(|&j| placements[j].col == c && spans[j].0 < end && spans[j].1 > start)
        };
        let free = (col + 1..cols).take_while(|&c| !blocked(c)).count();
        placements[i].span = 1 + free;
    }
    cluster.clear();
    column_ends.clear();
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

    fn p(col: usize, span: usize, cols: usize) -> Placement {
        Placement { col, span, cols }
    }

    #[test]
    fn separate_events_take_the_full_width() {
        assert_eq!(
            pack_columns(&[(0, 60), (60, 120), (200, 230)]),
            vec![p(0, 1, 1), p(0, 1, 1), p(0, 1, 1)]
        );
    }

    #[test]
    fn overlapping_events_sit_side_by_side() {
        assert_eq!(
            pack_columns(&[(0, 60), (30, 90)]),
            vec![p(0, 1, 2), p(1, 1, 2)]
        );
        // Three at once.
        assert_eq!(
            pack_columns(&[(0, 60), (0, 60), (0, 60)]),
            vec![p(0, 1, 3), p(1, 1, 3), p(2, 1, 3)]
        );
    }

    #[test]
    fn columns_are_reused_within_a_cluster() {
        // A long event with two short ones beside it, one after the other.
        assert_eq!(
            pack_columns(&[(0, 120), (0, 60), (60, 120)]),
            vec![p(0, 1, 2), p(1, 1, 2), p(1, 1, 2)]
        );
    }

    #[test]
    fn events_widen_into_free_columns() {
        // A and B overlap, B and C overlap: one cluster of two columns, with
        // C reusing A's column once A has ended.
        assert_eq!(
            pack_columns(&[(0, 60), (30, 120), (90, 150)]),
            vec![p(0, 1, 2), p(1, 1, 2), p(0, 1, 2)]
        );
        // G takes E's column once E ends, and widens over F's, free by then.
        assert_eq!(
            pack_columns(&[(0, 200), (0, 50), (0, 50), (60, 100)]),
            vec![p(0, 1, 3), p(1, 1, 3), p(2, 1, 3), p(1, 2, 3)]
        );
    }

    #[test]
    fn input_order_does_not_matter() {
        assert_eq!(
            pack_columns(&[(30, 90), (0, 60)]),
            vec![p(1, 1, 2), p(0, 1, 2)]
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
