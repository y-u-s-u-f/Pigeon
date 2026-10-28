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

/// Layout for one day of the time grid, like Apple Calendar's and
/// Fantastical's: events that overlap sit side by side in equal columns.
/// Each takes the first column free at its start, and widens over the
/// columns to its right that hold nothing it overlaps.
///
/// `spans` are `(start, end)` in any unit, end exclusive; the result is in
/// the same order. Events that overlap, directly or through others, form a
/// cluster that shares one column count.
pub fn pack_columns(spans: &[(i64, i64)]) -> Vec<Placement> {
    let mut order: Vec<usize> = (0..spans.len()).collect();
    order.sort_by_key(|&i| (spans[i].0, std::cmp::Reverse(spans[i].1)));

    let overlaps = |a: usize, b: usize| spans[a].0 < spans[b].1 && spans[b].0 < spans[a].1;

    let mut placements = vec![
        Placement {
            col: 0,
            span: 1,
            cols: 1,
        };
        spans.len()
    ];
    let mut cluster: Vec<usize> = Vec::new();
    let mut cluster_end = i64::MIN;

    for i in order {
        let (start, end) = spans[i];
        if start >= cluster_end && !cluster.is_empty() {
            finish(&mut placements, &mut cluster, &mut cluster_end, &overlaps);
        }
        // The first column without an event this one overlaps.
        let col = (0..)
            .find(|&c| {
                !cluster
                    .iter()
                    .any(|&j| placements[j].col == c && overlaps(i, j))
            })
            .expect("some column is free");
        placements[i].col = col;
        cluster.push(i);
        cluster_end = cluster_end.max(end);
    }
    if !cluster.is_empty() {
        finish(&mut placements, &mut cluster, &mut cluster_end, &overlaps);
    }
    placements
}

/// Give a finished cluster its column count, and widen each event over the
/// columns to its right that hold nothing it overlaps; then start the next
/// cluster empty.
fn finish(
    placements: &mut [Placement],
    cluster: &mut Vec<usize>,
    cluster_end: &mut i64,
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
                .any(|&j| placements[j].col == c && overlaps(i, j))
        };
        placements[i].span = 1 + (col + 1..cols).take_while(|&c| !blocked(c)).count();
    }
    cluster.clear();
    *cluster_end = i64::MIN;
}

/// Where a placed event goes across a day `width` wide: its left edge and
/// its width. Columns get an equal share; only when that share would be
/// narrower than `min` do they overlap, each at least `min` wide (or the
/// whole day, if that's narrower), stepped evenly across the day so the
/// last one ends at its edge.
pub fn column_box(placement: Placement, width: f32, min: f32) -> (f32, f32) {
    let cols = placement.cols as f32;
    let share = width / cols;
    if share >= min || placement.cols == 1 {
        return (placement.col as f32 * share, placement.span as f32 * share);
    }
    let min = min.min(width);
    let step = (width - min) / (cols - 1.);
    let left = placement.col as f32 * step;
    (left, (placement.span - 1) as f32 * step + min)
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
            pack_columns(&[(0, 60), (15, 90)]),
            vec![p(0, 1, 2), p(1, 1, 2)]
        );
        // Three at once.
        assert_eq!(
            pack_columns(&[(0, 60), (0, 60), (0, 60)]),
            vec![p(0, 1, 3), p(1, 1, 3), p(2, 1, 3)]
        );
        // One that starts much later still gets its own column, not a
        // cascade over the first.
        assert_eq!(
            pack_columns(&[(0, 180), (90, 150)]),
            vec![p(0, 1, 2), p(1, 1, 2)]
        );
    }

    #[test]
    fn freed_columns_are_reused() {
        // A runs long; B and C follow each other beside it.
        assert_eq!(
            pack_columns(&[(0, 180), (0, 60), (60, 120)]),
            vec![p(0, 1, 2), p(1, 1, 2), p(1, 1, 2)]
        );
    }

    #[test]
    fn events_widen_into_free_columns() {
        // A, B and C start together; D starts after B and C end, beside A,
        // and widens over both their columns.
        assert_eq!(
            pack_columns(&[(0, 180), (0, 60), (0, 60), (90, 150)]),
            vec![p(0, 1, 3), p(1, 1, 3), p(2, 1, 3), p(1, 2, 3)]
        );
        // C starts after A ends and takes A's column back; B, beside it,
        // keeps it from widening.
        assert_eq!(
            pack_columns(&[(0, 60), (30, 120), (90, 150)]),
            vec![p(0, 1, 2), p(1, 1, 2), p(0, 1, 2)]
        );
    }

    #[test]
    fn input_order_does_not_matter() {
        assert_eq!(
            pack_columns(&[(10, 90), (0, 60)]),
            vec![p(1, 1, 2), p(0, 1, 2)]
        );
    }

    #[test]
    fn columns_share_the_width_without_overlap() {
        assert_eq!(column_box(p(0, 1, 1), 300., 60.), (0., 300.));
        assert_eq!(column_box(p(1, 1, 3), 300., 60.), (100., 100.));
        assert_eq!(column_box(p(1, 2, 3), 300., 60.), (100., 200.));
    }

    #[test]
    fn narrow_columns_overlap_a_little() {
        // Four in 160: a share of 40 is too narrow, so each is 60 wide and
        // they step 100 / 3 apart; the last ends at the edge.
        let boxes: Vec<(f32, f32)> = (0..4).map(|c| column_box(p(c, 1, 4), 160., 60.)).collect();
        for &(left, width) in &boxes {
            assert!(width >= 60. - 0.01);
            assert!(left + width <= 160. + 0.01);
        }
        let (left, width) = boxes[3];
        assert!((left + width - 160.).abs() < 0.01);
        // Spanning every column still takes the whole day.
        let (left, width) = column_box(p(0, 4, 4), 160., 60.);
        assert_eq!(left, 0.);
        assert!((width - 160.).abs() < 0.01);
        // A day narrower than the minimum: everyone takes all of it.
        assert_eq!(column_box(p(1, 1, 2), 50., 60.), (0., 50.));
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
