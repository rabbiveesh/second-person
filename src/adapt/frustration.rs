//! Unproductive struggle, as opposed to the productive kind.
//!
//! Losing is part of a duel. Losing *fast*, or several rounds in a row, is the signal that the
//! player has stopped having fun.

/// Why we think the player is frustrated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrustrationSignal {
    /// Lost within [`QUICK_LOSS_SECS`] of the round starting, right after another loss. One quick
    /// loss alone is just charging in.
    QuickLoss,
    /// Lost [`LOSS_STREAK`] rounds in a row.
    LossStreak,
}

/// A loss this soon after the start counts as frustration.
pub const QUICK_LOSS_SECS: f32 = 20.0;
/// Losses in a row that count as frustration.
pub const LOSS_STREAK: usize = 3;

/// The first signal that fires for a finished round, if any. `losses_in_a_row` includes it.
pub fn detect(won: bool, secs: f32, losses_in_a_row: usize) -> Option<FrustrationSignal> {
    if won {
        None
    } else if losses_in_a_row >= LOSS_STREAK {
        Some(FrustrationSignal::LossStreak)
    } else if secs < QUICK_LOSS_SECS && losses_in_a_row >= 2 {
        Some(FrustrationSignal::QuickLoss)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thresholds() {
        assert_eq!(detect(true, 1.0, 0), None);
        assert_eq!(detect(false, 60.0, 2), None);
        assert_eq!(detect(false, 60.0, 3), Some(FrustrationSignal::LossStreak));
        assert_eq!(detect(false, 5.0, 1), None, "one quick loss is just charging in");
        assert_eq!(detect(false, 5.0, 2), Some(FrustrationSignal::QuickLoss));
    }
}
