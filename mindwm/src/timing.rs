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

/// Whether a frame aimed at the vblank at `target` and actually shown at
/// `presented` missed the vblank it was drawn for.
///
/// The tempting test -- was the gap since the previous frame longer than one
/// refresh -- answers a different question. An output only presents when
/// something changed, so a 60 Hz video on a 240 Hz panel leaves three empty
/// vblanks between every pair of frames by design, and counting those as
/// misses reports a third of a perfectly smooth desktop as stutter. Worse,
/// it reports it in exactly the situation the number would be consulted in,
/// which makes a real stutter impossible to see next to it.
///
/// What matters is whether the frame reached the screen at the vblank the
/// compositor drew it for. An on-time frame lands within jitter of `target`;
/// one that missed a vblank lands a whole refresh past it. Half a refresh of
/// slack sits between the two.
pub fn missed_vblank(target: Duration, presented: Duration, frame: Duration) -> bool {
    presented.saturating_sub(target) > frame / 2
}

const FLIP_DEADLINE_FRAMES: u32 = 30;
const FLIP_DEADLINE_MIN: Duration = Duration::from_secs(1);

/// How many recent repaints are kept to decide how long the next one will
/// take. Long enough to cover a burst of heavy frames, short enough to come
/// back down once the desktop is quiet again.
const RENDER_HISTORY: usize = 32;

/// Slack on top of the measured repaint time. The kernel has to accept the
/// atomic commit before the vblank as well, and a frame that misses is worth
/// far more than the fraction of a millisecond it saves.
const RENDER_SAFETY: Duration = Duration::from_micros(500);

/// How long the compositor's own repaint takes on one output, over the last
/// few frames.
///
/// The repaint is deliberately started *late*, so that clients get as much of
/// the frame as possible to draw in (see the note at the call site). How late
/// it can start without missing the vblank depends on how long the repaint
/// itself takes, which is a property of the display, the scene and the GPU,
/// not a constant: 4K at 240 Hz leaves 4.17 ms for everything.
#[derive(Debug, Default, Clone)]
pub struct RenderTimes {
    recent: [Duration; RENDER_HISTORY],
    next: usize,
    filled: usize,
    /// The worst repaint ever seen on this output, for the record.
    pub worst: Duration,
    /// The most recent one.
    pub last: Duration,
}

impl RenderTimes {
    pub fn push(&mut self, took: Duration) {
        self.recent[self.next] = took;
        self.next = (self.next + 1) % RENDER_HISTORY;
        self.filled = (self.filled + 1).min(RENDER_HISTORY);
        self.worst = self.worst.max(took);
        self.last = took;
    }

    /// What to budget for the next repaint: the worst of the recent ones, so
    /// one heavy frame in thirty-two still gets its time. Zero until an
    /// output has drawn anything, which asks for the old fixed delay.
    pub fn estimate(&self) -> Duration {
        self.recent[..self.filled].iter().copied().max().unwrap_or_default()
    }
}

/// How long after a vblank to start the repaint that should reach the *next*
/// vblank.
///
/// Waiting is what keeps latency down: a client driven by frame callbacks
/// only draws once the compositor has, so every millisecond the compositor
/// waits is a millisecond fresher the frame on screen is. Anvil waits a flat
/// 0.6 of a frame, which is 2.5 ms at 240 Hz and leaves 1.7 ms to render,
/// commit and have the kernel take it. Miss that and the frame lands a whole
/// refresh late, which is what stutter is.
///
/// So: leave room for a repaint as slow as the slowest recent one, and never
/// start later than anvil's 0.6 would have. The floor keeps a heavy output
/// from repainting flat out with no gap for clients at all.
pub fn repaint_delay(frame_duration: Duration, estimate: Duration) -> Duration {
    let latest = frame_duration.mul_f64(0.6);
    let floor = frame_duration.mul_f64(0.1);
    frame_duration
        .saturating_sub(estimate.saturating_add(RENDER_SAFETY))
        .clamp(floor, latest)
}

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
    fn a_heavy_output_starts_its_repaint_earlier() {
        let frame = frame_duration(240_000); // 4.17 ms
        // Nothing measured yet: anvil's flat 0.6 of a frame.
        assert_eq!(repaint_delay(frame, Duration::ZERO), frame.mul_f64(0.6));
        // A repaint that takes 2 ms cannot start at 2.5 ms and make it.
        let delay = repaint_delay(frame, Duration::from_millis(2));
        assert!(delay < frame.mul_f64(0.6));
        assert!(delay + Duration::from_millis(2) + RENDER_SAFETY <= frame);
        // A repaint slower than the frame itself still leaves clients a
        // tenth of the frame rather than spinning.
        assert_eq!(repaint_delay(frame, Duration::from_millis(50)), frame.mul_f64(0.1));
    }

    #[test]
    fn the_estimate_follows_the_worst_recent_frame() {
        let mut times = RenderTimes::default();
        assert_eq!(times.estimate(), Duration::ZERO);
        times.push(Duration::from_millis(1));
        times.push(Duration::from_millis(3));
        times.push(Duration::from_millis(2));
        assert_eq!(times.estimate(), Duration::from_millis(3));
        assert_eq!(times.last, Duration::from_millis(2));
        assert_eq!(times.worst, Duration::from_millis(3));
        // Once the heavy frame has aged out of the window, the estimate comes
        // back down and latency with it.
        for _ in 0..RENDER_HISTORY {
            times.push(Duration::from_millis(1));
        }
        assert_eq!(times.estimate(), Duration::from_millis(1));
        assert_eq!(times.worst, Duration::from_millis(3));
    }

    #[test]
    fn invalid_refresh_has_a_safe_fallback() {
        assert_eq!(frame_duration(0), frame_duration(60_000));
        assert_eq!(frame_duration(-1), frame_duration(60_000));
    }

    /// A 60 Hz video on a 240 Hz panel presents one frame in four, every one
    /// of them exactly when it was meant to. None of them is late.
    #[test]
    fn a_slow_client_on_a_fast_panel_is_not_stutter() {
        let frame = frame_duration(240_000);
        let mut target = Duration::from_secs(10);
        for _ in 0..100 {
            // Four vblanks between frames, each landing on the one aimed at.
            assert!(!missed_vblank(target, target + Duration::from_micros(80), frame));
            target += frame * 4;
        }
    }

    /// A frame that reached the screen a whole refresh after the vblank it
    /// was drawn for is the thing the number is for.
    #[test]
    fn a_frame_that_missed_its_vblank_is_late() {
        let frame = frame_duration(240_000);
        let target = Duration::from_secs(10);
        assert!(missed_vblank(target, target + frame, frame));
        assert!(missed_vblank(target, target + frame * 3, frame));
    }

    /// The vblank a flip lands on is never exactly the time that was aimed
    /// at, and a little jitter either way is not a missed frame.
    #[test]
    fn jitter_around_the_vblank_is_not_late() {
        let frame = frame_duration(240_000);
        let target = Duration::from_secs(10);
        assert!(!missed_vblank(target, target, frame));
        assert!(!missed_vblank(target, target + frame / 3, frame));
        // Early is never late.
        assert!(!missed_vblank(target, target.saturating_sub(frame), frame));
    }
}
