/// A simple fragmentation score: number of distinct app switches per minute,
/// normalized to roughly 0..1 (1 = highly fragmented).
pub fn score(switches: u32, observed_seconds: u32) -> f32 {
    if observed_seconds == 0 {
        return 0.0;
    }
    let per_minute = (switches as f32) / (observed_seconds as f32 / 60.0);
    // 6 switches/min already feels chaotic, so cap there.
    (per_minute / 6.0).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_when_no_observations() {
        assert_eq!(score(10, 0), 0.0);
    }

    #[test]
    fn caps_at_one() {
        assert!((score(100, 60) - 1.0).abs() < f32::EPSILON);
    }
}
