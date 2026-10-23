//! 内存状态的判定：平静 / 紧张 / 告急，带「回差」避免在临界值附近来回跳。

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Normal,
    Warn,
    Alert,
}

/// 告急后降到「告急线 - 5%」以下才解除。
const ALERT_HYSTERESIS: f32 = 5.0;
/// 紧张后降到「提醒线 - 3%」以下才解除。
const WARN_HYSTERESIS: f32 = 3.0;

pub fn next_level(cur: Level, p: f32, warn: u32, alert: u32) -> Level {
    let (w, a) = (warn as f32, alert as f32);
    match cur {
        Level::Alert if p >= a - ALERT_HYSTERESIS => Level::Alert,
        Level::Alert | Level::Warn if p >= a => Level::Alert,
        Level::Alert | Level::Warn if p >= w - WARN_HYSTERESIS => Level::Warn,
        Level::Normal if p >= a => Level::Alert,
        Level::Normal if p >= w => Level::Warn,
        _ => Level::Normal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_have_hysteresis() {
        let l = next_level(Level::Normal, 74.0, 75, 85);
        assert_eq!(l, Level::Normal);
        let l = next_level(l, 75.0, 75, 85);
        assert_eq!(l, Level::Warn);
        // 降到 72% 以上仍然保持紧张
        assert_eq!(next_level(l, 72.5, 75, 85), Level::Warn);
        assert_eq!(next_level(l, 71.0, 75, 85), Level::Normal);
        let l = next_level(l, 86.0, 75, 85);
        assert_eq!(l, Level::Alert);
        // 告急后要降到 80% 以下才解除
        assert_eq!(next_level(l, 81.0, 75, 85), Level::Alert);
        assert_eq!(next_level(l, 79.0, 75, 85), Level::Warn);
        assert_eq!(next_level(l, 60.0, 75, 85), Level::Normal);
        assert_eq!(next_level(Level::Normal, 90.0, 75, 85), Level::Alert);
    }
}
