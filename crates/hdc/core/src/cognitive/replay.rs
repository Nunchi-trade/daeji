//! Mattar-Daw replay — prioritizes which memories to replay during NREM.

/// An entry eligible for replay prioritization.
#[derive(Clone, Debug)]
pub struct ReplayEntry {
    /// Index into the knowledge store.
    pub store_index: usize,
    /// Prediction error: |predicted_reward - actual_reward|.
    pub prediction_error: f64,
    /// How often similar situations have occurred recently.
    pub recurrence_count: u32,
    /// Discount factor for future relevance (0.0-1.0).
    pub discount_factor: f64,
}

/// Compute the Expected Value of Backup for a single replay candidate.
///
/// EVB = gain * need
///   gain = prediction_error  (how surprising was the outcome?)
///   need = recurrence_count * discount_factor  (how relevant going forward?)
///
/// NOTE: This is a 2-factor product. Do NOT add a third factor.
pub fn mattar_daw_evb(entry: &ReplayEntry) -> f64 {
    let gain = entry.prediction_error;
    let need = entry.recurrence_count as f64 * entry.discount_factor;
    gain * need
}

/// Sort replay candidates by descending EVB and return the top N.
pub fn prioritize_replay(candidates: &mut Vec<ReplayEntry>, max_replay: usize) -> Vec<ReplayEntry> {
    candidates.sort_by(|a, b| {
        let evb_a = mattar_daw_evb(a);
        let evb_b = mattar_daw_evb(b);
        evb_b.partial_cmp(&evb_a).unwrap_or(std::cmp::Ordering::Equal)
    });
    candidates.truncate(max_replay);
    candidates.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evb_is_two_factor_product() {
        let entry = ReplayEntry {
            store_index: 0,
            prediction_error: 0.5,
            recurrence_count: 10,
            discount_factor: 0.8,
        };
        let evb = mattar_daw_evb(&entry);
        // gain = 0.5, need = 10 * 0.8 = 8.0, EVB = 0.5 * 8.0 = 4.0
        assert!((evb - 4.0).abs() < 1e-9);
    }

    #[test]
    fn evb_zero_prediction_error() {
        let entry = ReplayEntry {
            store_index: 0,
            prediction_error: 0.0,
            recurrence_count: 100,
            discount_factor: 1.0,
        };
        assert!((mattar_daw_evb(&entry) - 0.0).abs() < 1e-9);
    }

    #[test]
    fn evb_zero_recurrence() {
        let entry = ReplayEntry {
            store_index: 0,
            prediction_error: 1.0,
            recurrence_count: 0,
            discount_factor: 1.0,
        };
        assert!((mattar_daw_evb(&entry) - 0.0).abs() < 1e-9);
    }

    #[test]
    fn prioritize_sorts_by_descending_evb() {
        let mut candidates = vec![
            ReplayEntry {
                store_index: 0,
                prediction_error: 0.1,
                recurrence_count: 1,
                discount_factor: 1.0,
            },
            ReplayEntry {
                store_index: 1,
                prediction_error: 1.0,
                recurrence_count: 10,
                discount_factor: 0.9,
            },
            ReplayEntry {
                store_index: 2,
                prediction_error: 0.5,
                recurrence_count: 5,
                discount_factor: 0.5,
            },
        ];
        let result = prioritize_replay(&mut candidates, 10);
        // EVBs: 0.1, 9.0, 1.25 -> sorted: [9.0, 1.25, 0.1]
        assert_eq!(result[0].store_index, 1);
        assert_eq!(result[1].store_index, 2);
        assert_eq!(result[2].store_index, 0);
    }

    #[test]
    fn prioritize_truncates_to_max() {
        let mut candidates = vec![
            ReplayEntry {
                store_index: 0,
                prediction_error: 0.1,
                recurrence_count: 1,
                discount_factor: 1.0,
            },
            ReplayEntry {
                store_index: 1,
                prediction_error: 1.0,
                recurrence_count: 10,
                discount_factor: 0.9,
            },
            ReplayEntry {
                store_index: 2,
                prediction_error: 0.5,
                recurrence_count: 5,
                discount_factor: 0.5,
            },
        ];
        let result = prioritize_replay(&mut candidates, 2);
        assert_eq!(result.len(), 2);
    }
}
