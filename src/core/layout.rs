//! Fit all rows to the window.

/// What the fit needs to know about one visible row.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum FitRow {
    /// Minimum height of the signal area, plus the analyzer lanes.
    Digital {
        min: f32,
        lanes: f32,
    },
    Analog {
        min: f32,
    },
    /// Fixed height.
    Analyzer {
        height: f32,
    },
}

/// Heights for `rows` filling `available` points (the graph
/// height minus the scrollbar and spacing). Digital heights exclude their
/// lanes, as stored row heights do.
///
/// 1. analyzer rows keep their height;
/// 2. with analog rows visible, digital rows get their minimum;
/// 3. otherwise digital rows share the space equally, between their
///    minimum and `max(200, 50 % of window_height)`;
/// 4. analog rows share what is left, each at least its minimum; the last
///    one absorbs the rounding remainder.
pub fn fit_rows(rows: &[FitRow], available: f32, window_height: f32) -> Vec<f32> {
    let mut heights = vec![0.0f32; rows.len()];
    let mut left = available;
    let analog: Vec<usize> = (0..rows.len()).filter(|&i| matches!(rows[i], FitRow::Analog { .. })).collect();
    let digital: Vec<usize> = (0..rows.len()).filter(|&i| matches!(rows[i], FitRow::Digital { .. })).collect();
    for (i, r) in rows.iter().enumerate() {
        if let FitRow::Analyzer { height } = *r {
            heights[i] = height;
            left -= height;
        }
    }
    let max_digital = 200f32.max(window_height * 0.5);
    if !digital.is_empty() {
        let lanes: f32 = digital.iter().map(|&i| lanes_of(rows[i])).sum();
        let share = if analog.is_empty() { (left - lanes) / digital.len() as f32 } else { 0.0 };
        for &i in &digital {
            let FitRow::Digital { min, lanes } = rows[i] else { unreachable!() };
            let h = share.floor().clamp(min, max_digital.max(min));
            heights[i] = h;
            left -= h + lanes;
        }
        // Digital-only: the last row absorbs the remainder, within its limits.
        if analog.is_empty()
            && let Some(&last) = digital.last()
            && let FitRow::Digital { min, .. } = rows[last]
        {
            let h = (heights[last] + left).clamp(min, max_digital.max(min));
            left -= h - heights[last];
            heights[last] = h;
        }
    }
    if !analog.is_empty() {
        let share = (left / analog.len() as f32).floor();
        for &i in &analog {
            let FitRow::Analog { min } = rows[i] else { unreachable!() };
            heights[i] = share.max(min);
        }
        let used: f32 = analog.iter().map(|&i| heights[i]).sum();
        let last = *analog.last().unwrap();
        let FitRow::Analog { min } = rows[last] else { unreachable!() };
        heights[last] = (heights[last] + left - used).max(min);
    }
    heights
}

fn lanes_of(r: FitRow) -> f32 {
    match r {
        FitRow::Digital { lanes, .. } => lanes,
        _ => 0.0,
    }
}

/// U-13: fit-to-window distribution.
#[cfg(test)]
mod tests {
    use super::*;

    const D: FitRow = FitRow::Digital { min: 20.0, lanes: 0.0 };
    const A: FitRow = FitRow::Analog { min: 45.0 };

    #[test]
    fn digital_only_shares_equally_with_remainder_last() {
        let h = fit_rows(&[D, D, D], 301.0, 800.0);
        assert_eq!(h, [100.0, 100.0, 101.0]);
        assert_eq!(h.iter().sum::<f32>(), 301.0);
    }

    #[test]
    fn digital_only_is_capped_and_floored() {
        // max(200, 50 % of 800) = 400 per row.
        assert_eq!(fit_rows(&[D, D], 1000.0, 800.0), [400.0, 400.0]);
        // Too little room: each keeps its minimum (the stack scrolls).
        assert_eq!(fit_rows(&[D, D, D], 30.0, 800.0), [20.0, 20.0, 20.0]);
    }

    #[test]
    fn mixed_gives_digital_its_minimum_and_analog_the_rest() {
        let h = fit_rows(&[D, D, A, A], 501.0, 800.0);
        assert_eq!(h, [20.0, 20.0, 230.0, 231.0]);
        assert_eq!(h.iter().sum::<f32>(), 501.0);
    }

    #[test]
    fn lanes_and_analyzer_rows_take_their_space_first() {
        let lanes = FitRow::Digital { min: 20.0, lanes: 40.0 };
        let analyzer = FitRow::Analyzer { height: 40.0 };
        // 400 − 40 (analyzer) − 40 (lanes) = 320 for two digital rows.
        let h = fit_rows(&[lanes, D, analyzer], 400.0, 800.0);
        assert_eq!(h, [160.0, 160.0, 40.0]);
        // Mixed: analog gets 400 − 40 − (20 + 40) − 20 = 280.
        assert_eq!(fit_rows(&[lanes, D, analyzer, A], 400.0, 800.0), [20.0, 20.0, 40.0, 280.0]);
    }

    #[test]
    fn analog_rows_keep_their_minimum() {
        assert_eq!(fit_rows(&[A, A], 60.0, 800.0), [45.0, 45.0]);
    }
}
