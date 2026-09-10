//! Frame deadlines use the output's millihertz refresh rate without rounding
//! down to whole milliseconds or catching up missed frames in a busy loop.

use std::time::Duration;

pub fn frame_duration(refresh_millihz: i32) -> Duration {
    let refresh = if refresh_millihz > 0 { refresh_millihz as u64 } else { 60_000 };
    Duration::from_nanos(1_000_000_000_000 / refresh)
}

pub fn next_repaint(now: Duration, previous_target: Duration, refresh_millihz: i32) -> Duration {
    now.max(previous_target).saturating_add(frame_duration(refresh_millihz))
}

/// How long to wait for the page flip just queued before deciding that its
/// vblank is never coming. Thirty frames, and never less than a second: KWin
/// waits the same second and says why, that it "should always be longer than
/// any real pageflip can take, even with PSR and modesets". Deciding too
/// early costs a visible reset of a display that was merely slow.
pub fn flip_deadline(refresh_millihz: Option<i32>) -> Duration {
    refresh_millihz
        .map(|refresh| frame_duration(refresh) * FLIP_DEADLINE_FRAMES)
        .unwrap_or(FLIP_DEADLINE_MIN)
        .max(FLIP_DEADLINE_MIN)
}

const FLIP_DEADLINE_FRAMES: u32 = 30;
const FLIP_DEADLINE_MIN: Duration = Duration::from_secs(1);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_flip_gets_at_least_a_second_at_every_refresh_rate() {
        // 30 frames at 240 Hz is an eighth of a second: far too eager.
        assert_eq!(flip_deadline(Some(240_000)), Duration::from_secs(1));
        assert_eq!(flip_deadline(Some(60_000)), Duration::from_secs(1));
        assert_eq!(flip_deadline(None), Duration::from_secs(1));
        // A slow display gets proportionally longer.
        assert_eq!(flip_deadline(Some(24_000)), frame_duration(24_000) * 30);
        assert!(flip_deadline(Some(24_000)) > Duration::from_millis(1249));
    }

    #[test]
    fn fractional_refresh_keeps_submillisecond_precision() {
        assert_eq!(frame_duration(144_000), Duration::from_nanos(6_944_444));
        assert_eq!(frame_duration(239_760), Duration::from_nanos(4_170_837));
        assert_eq!(frame_duration(60_000), Duration::from_nanos(16_666_666));
    }

    #[test]
    fn overdue_frames_do_not_create_immediate_catchup_repaints() {
        let now = Duration::from_secs(10);
        let next = next_repaint(now, Duration::from_secs(1), 144_000);
        assert_eq!(next - now, frame_duration(144_000));
        let future = now + frame_duration(144_000);
        assert_eq!(next_repaint(now, future, 144_000), future + frame_duration(144_000));
    }

    #[test]
    fn invalid_refresh_has_a_safe_fallback() {
        assert_eq!(frame_duration(0), frame_duration(60_000));
        assert_eq!(frame_duration(-1), frame_duration(60_000));
    }
}
