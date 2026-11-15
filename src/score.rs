//! Points: speed and streaks for multiple choice, truth and fooled players for bluffs.

/// Every correct answer is worth at least this.
pub const BASE: u32 = 500;
/// Up to this much more for answering at once.
pub const SPEED: u32 = 500;
/// Each answer in a row after the first adds this, up to [`STREAK_CAP`].
pub const STREAK_STEP: u32 = 100;
pub const STREAK_CAP: u32 = 400;
/// Bluff: picking the truth.
pub const TRUTH: u32 = 1000;
/// Bluff: for each player your fake fooled.
pub const FOOLED: u32 = 500;

/// Points for a correct answer with `left` of `limit` seconds remaining, `streak` correct answers
/// in a row counting this one.
pub fn points(
    left: f32,
    limit: f32,
    streak: u32,
    multiplier: u32,
) -> u32 {
    let share = if limit > 0.0 {
        (left / limit).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let speed = ((SPEED as f32 * share) / 10.0).round() as u32 * 10;
    let bonus = (STREAK_STEP * streak.saturating_sub(1)).min(STREAK_CAP);
    (BASE + speed + bonus) * multiplier
}

/// Streak bonus alone, for the reveal's flame label.
pub fn streak_bonus(streak: u32) -> u32 {
    (STREAK_STEP * streak.saturating_sub(1)).min(STREAK_CAP)
}

/// Points for one bluff question.
pub fn bluff(
    found_truth: bool,
    fooled: u32,
    multiplier: u32,
) -> u32 {
    (u32::from(found_truth) * TRUTH + fooled * FOOLED) * multiplier
}

/// Places for scores in the given order, ties sharing a place: [900, 900, 500] -> [1, 1, 3].
pub fn places(scores: &[u32]) -> Vec<usize> {
    scores
        .iter()
        .map(|s| 1 + scores.iter().filter(|o| *o > s).count())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speed_runs_from_base_to_double() {
        assert_eq!(points(0.0, 20.0, 1, 1), 500);
        assert_eq!(points(20.0, 20.0, 1, 1), 1000);
        assert_eq!(points(10.0, 20.0, 1, 1), 750);
        assert_eq!(points(25.0, 20.0, 1, 1), 1000);
        assert_eq!(points(5.0, 0.0, 1, 1), 500);
        // Rounded to tens
        assert_eq!(points(7.0, 20.0, 1, 1) % 10, 0);
    }

    #[test]
    fn streaks_cap_and_multipliers_apply() {
        assert_eq!(points(0.0, 20.0, 2, 1), 600);
        assert_eq!(points(0.0, 20.0, 5, 1), 900);
        assert_eq!(points(0.0, 20.0, 50, 1), 900);
        assert_eq!(points(20.0, 20.0, 1, 2), 2000);
        assert_eq!(streak_bonus(0), 0);
        assert_eq!(streak_bonus(3), 200);
    }

    #[test]
    fn bluffs_and_ties() {
        assert_eq!(bluff(true, 0, 1), 1000);
        assert_eq!(bluff(false, 3, 1), 1500);
        assert_eq!(bluff(true, 2, 2), 4000);
        assert_eq!(places(&[900, 900, 500]), vec![1, 1, 3]);
        assert_eq!(places(&[1, 3, 2]), vec![3, 1, 2]);
        assert_eq!(places(&[]), Vec::<usize>::new());
    }
}
