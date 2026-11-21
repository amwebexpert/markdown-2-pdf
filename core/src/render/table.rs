//! Table column sizing: a predictable, content-aware width per column.

/// What one column needs, both including cell padding.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Demand {
    /// Widest unbreakable word: below this the column must char-split words.
    pub min: f32,
    /// Widest cell laid out on a single line: above this is wasted space.
    pub max: f32,
}

/// Column widths that never sum past `available`:
///
/// 1. everything fits on one line → each column gets its `max`;
/// 2. every column's longest word fits → each gets its `min`, and the slack is
///    shared in proportion to how much more each column wants (`max − min`);
/// 3. otherwise → `available` split in proportion to `min` (words char-wrap;
///    the font is never shrunk).
pub(super) fn column_widths(demands: &[Demand], available: f32) -> Vec<f32> {
    let sum_max: f32 = demands.iter().map(|d| d.max).sum();
    if sum_max <= available {
        return demands.iter().map(|d| d.max).collect();
    }
    let sum_min: f32 = demands.iter().map(|d| d.min).sum();
    if sum_min <= available {
        let slack = available - sum_min;
        let want: f32 = demands.iter().map(|d| d.max - d.min).sum();
        return demands
            .iter()
            .map(|d| d.min + slack * (d.max - d.min) / want)
            .collect();
    }
    demands
        .iter()
        .map(|d| available * d.min / sum_min)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn demand(min: f32, max: f32) -> Demand {
        Demand { min, max }
    }

    fn assert_close(actual: &[f32], expected: &[f32]) {
        assert_eq!(actual.len(), expected.len());
        for (a, e) in actual.iter().zip(expected) {
            assert!((a - e).abs() < 1e-3, "{actual:?} != {expected:?}");
        }
    }

    #[test]
    fn narrow_table_keeps_natural_widths() {
        let widths = column_widths(&[demand(20.0, 50.0), demand(30.0, 80.0)], 400.0);
        assert_close(&widths, &[50.0, 80.0]);
    }

    #[test]
    fn slack_goes_to_the_columns_that_want_more() {
        // Σmin = 100, slack = 100; wants 0 and 300 → all slack to column 2.
        let widths = column_widths(&[demand(40.0, 40.0), demand(60.0, 360.0)], 200.0);
        assert_close(&widths, &[40.0, 160.0]);
        // Wants 100 and 300 → slack split 1:3.
        let widths = column_widths(&[demand(40.0, 140.0), demand(60.0, 360.0)], 200.0);
        assert_close(&widths, &[65.0, 135.0]);
    }

    #[test]
    fn a_long_word_column_is_never_squeezed_below_its_word() {
        // Proportional shrink of max (old behaviour) would give the URL column
        // 300 × 200/600 = 100 < its 150 pt word.
        let widths = column_widths(&[demand(150.0, 150.0), demand(20.0, 450.0)], 300.0);
        assert!(widths[0] >= 150.0 - 1e-3, "{widths:?}");
    }

    #[test]
    fn overflowing_words_split_the_width_by_their_size() {
        let widths = column_widths(&[demand(300.0, 300.0), demand(100.0, 500.0)], 200.0);
        assert_close(&widths, &[150.0, 50.0]);
    }

    #[test]
    fn widths_never_exceed_the_available_space() {
        let cases = [
            vec![demand(10.0, 10.0)],
            vec![
                demand(10.0, 900.0),
                demand(10.0, 900.0),
                demand(10.0, 900.0),
            ],
            vec![demand(500.0, 500.0), demand(500.0, 600.0)],
            vec![demand(50.0, 50.0), demand(50.0, 50.0)],
        ];
        for demands in cases {
            let sum: f32 = column_widths(&demands, 468.0).iter().sum();
            assert!(sum <= 468.0 + 1e-3, "{demands:?} → {sum}");
        }
    }
}
