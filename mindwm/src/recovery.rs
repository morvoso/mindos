// Copyright (c) 2026 Black Arrow Software, LLC. All rights reserved.
// Licensed under the MindOS Software License; see LICENSE at the repository root.

//! What the DRM backend does about a display that stops taking frames.
//!
//! A display freezes in two ways. A page flip goes out and its vblank never
//! comes back: NVIDIA's driver does this when it fails a commit after
//! queueing its flip (it logs "Failed to apply atomic modeset" and still
//! reports success), or when a buffer on the plane waits on a fence that
//! never signals. Or the kernel refuses frame after frame: a plane lost its
//! framebuffer, the mode blob went stale across a suspend, the CRTC's state
//! no longer matches what the compositor believes.
//!
//! Neither clears up by waiting. Frames are retried briefly with some
//! backoff, and then the output is reset, one step harder each time it does
//! not come back ([`Step`]), with longer pauses once the gentle steps are
//! used up so a display that cannot be lit is not flashed every second.
//! Every step starts the same way: a helper thread makes the kernel let go of
//! whatever it still holds for the CRTC, so the wait for a stuck flip, up to
//! three seconds on NVIDIA, never freezes the other displays or the pointer.
//!
//! This module is the policy, free of any DRM state, so it can be tested.

use std::io;
use std::time::Duration;

use smithay::backend::{drm::DrmError, SwapBuffersError};

/// Why the kernel did not take a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    /// A commit is still in flight on the CRTC (EBUSY).
    Busy,
    /// The device is not ours at the moment (EACCES, EPERM): another
    /// process holds DRM master, or a session switch is under way.
    Permission,
    /// The kernel's check of the whole CRTC setup failed.
    TestFailed,
    /// The kernel refused the commit (EINVAL, ENOSPC, ...).
    Rejected,
    /// Out of buffers, or another condition expected to pass.
    Temporary,
    /// The renderer or the allocator failed.
    Renderer,
}

impl Failure {
    /// Classify a failed frame. `None` is a paused session, which is not a
    /// failure at all: resuming redraws everything.
    pub fn of(err: &SwapBuffersError) -> Option<Failure> {
        let (inner, temporary) = match err {
            SwapBuffersError::AlreadySwapped => return Some(Failure::Temporary),
            SwapBuffersError::TemporaryFailure(inner) => (inner, true),
            SwapBuffersError::ContextLost(inner) => (inner, false),
        };
        if let Some(drm) = inner.downcast_ref::<DrmError>() {
            return match drm {
                DrmError::DeviceInactive => None,
                DrmError::DrmMasterFailed => Some(Failure::Permission),
                DrmError::TestFailed(_) => Some(Failure::TestFailed),
                DrmError::Access(access) => Some(Failure::of_io(&access.source)),
                _ => Some(Failure::Rejected),
            };
        }
        if let Some(io) = inner.downcast_ref::<io::Error>() {
            return Some(Failure::of_io(io));
        }
        Some(if temporary { Failure::Temporary } else { Failure::Renderer })
    }

    fn of_io(err: &io::Error) -> Failure {
        match err.raw_os_error() {
            Some(libc::EBUSY) => Failure::Busy,
            Some(libc::EACCES) | Some(libc::EPERM) => Failure::Permission,
            Some(libc::EAGAIN) | Some(libc::EINTR) => Failure::Temporary,
            _ => Failure::Rejected,
        }
    }
}

/// How hard one recovery goes. Each starts with the CRTC released.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Forget the frame that never came back and draw the whole screen
    /// again. Enough for a lost vblank.
    Resubmit,
    /// Read the CRTC's state back from the kernel, so the next frame is a
    /// full commit of everything rather than a flip of what changed.
    ResetState,
    /// Switch the output off and set its mode again, with a new mode blob:
    /// what unplugging and replugging the monitor would do.
    Relight,
    /// Every output on the device off, and each lit again, the most
    /// demanding first: for an output that cannot be lit next to the others.
    ResetDevice,
}

impl Step {
    /// The step for the `recoveries`-th recovery since the output last worked.
    pub fn of(recoveries: u8) -> Step {
        match recoveries {
            0 | 1 => Step::Resubmit,
            2 => Step::ResetState,
            3 => Step::Relight,
            _ => Step::ResetDevice,
        }
    }

    /// The number of recoveries at which this step is taken.
    pub fn recoveries(self) -> u8 {
        match self {
            Step::Resubmit => 1,
            Step::ResetState => 2,
            Step::Relight => 3,
            Step::ResetDevice => 4,
        }
    }

    pub fn describe(self) -> &'static str {
        match self {
            Step::Resubmit => "drawing the whole screen again",
            Step::ResetState => "re-reading its state from the kernel",
            Step::Relight => "switching it off and setting its mode again",
            Step::ResetDevice => "switching every display on the GPU off and on again",
        }
    }
}

/// What started a recovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cause {
    /// A page flip went out and its vblank never came back.
    StuckFlip,
    /// The kernel refused frames for [`REFUSED_FOR`].
    Refused(Failure),
    /// Someone asked for it: the reset key, or the `reset_displays` request.
    Requested,
    /// Another output on the same GPU needs the whole device reset.
    Device,
}

impl Cause {
    /// The gentlest step worth taking for this cause, however well the output
    /// was doing before. A lost vblank may need nothing more than a new
    /// frame. Frames the kernel kept refusing were already retried, and the
    /// state re-read after the first refusal, so a refused commit starts
    /// with a new mode: a mode blob that went stale across a suspend is
    /// refused forever otherwise. Someone who asks for a reset has a display
    /// that looks wrong in a way the compositor cannot see, like a monitor
    /// that lost its link, and wants it lit again.
    pub fn first_step(self) -> Step {
        match self {
            Cause::StuckFlip => Step::Resubmit,
            Cause::Refused(Failure::TestFailed) | Cause::Refused(Failure::Rejected) => Step::Relight,
            Cause::Refused(_) => Step::ResetState,
            Cause::Requested => Step::Relight,
            Cause::Device => Step::ResetDevice,
        }
    }
}

/// How long after the previous recovery the `recoveries`-th may start. The
/// first four follow each other as fast as they fail; after that the output
/// is evidently not coming back soon, and trying every second would flash
/// every display on the GPU every second.
pub fn backoff(recoveries: u8) -> Duration {
    Duration::from_secs(match recoveries {
        0..=4 => 0,
        5 => 5,
        6 => 15,
        7 => 30,
        _ => 60,
    })
}

/// How long to wait before trying again after the `refused`-th frame in a
/// row the kernel did not take: a frame for a hiccup, then slower, so a
/// display that keeps refusing does not spin the compositor.
pub fn retry_delay(refused: u32, frame: Duration) -> Duration {
    match refused {
        0..=3 => frame,
        4..=10 => frame.max(Duration::from_millis(50)),
        _ => frame.max(Duration::from_millis(250)),
    }
}

/// Refused frames retried for this long end in a recovery.
pub const REFUSED_FOR: Duration = Duration::from_secs(2);
/// An output that shows frames this long after a recovery is well again: its
/// next incident starts from the first step.
pub const SETTLE: Duration = Duration::from_secs(10);
/// How often a run of refused frames is logged above debug level.
pub const REFUSALS_LOGGED_EVERY: Duration = Duration::from_secs(5);

#[cfg(test)]
mod tests {
    use super::*;
    use smithay::reexports::drm::control::crtc;

    fn io_err(errno: i32) -> SwapBuffersError {
        SwapBuffersError::ContextLost(Box::new(io::Error::from_raw_os_error(errno)))
    }

    #[test]
    fn failures_are_classified_by_what_the_kernel_said() {
        assert_eq!(Failure::of(&io_err(libc::EINVAL)), Some(Failure::Rejected));
        assert_eq!(Failure::of(&io_err(libc::EBUSY)), Some(Failure::Busy));
        assert_eq!(Failure::of(&io_err(libc::EACCES)), Some(Failure::Permission));
        let inactive = SwapBuffersError::TemporaryFailure(Box::new(DrmError::DeviceInactive));
        assert_eq!(Failure::of(&inactive), None);
        let crtc = smithay::reexports::drm::control::from_u32::<crtc::Handle>(42).unwrap();
        let test = SwapBuffersError::ContextLost(Box::new(DrmError::TestFailed(crtc)));
        assert_eq!(Failure::of(&test), Some(Failure::TestFailed));
        let other = SwapBuffersError::ContextLost("the GPU went away".into());
        assert_eq!(Failure::of(&other), Some(Failure::Renderer));
        let slots = SwapBuffersError::TemporaryFailure("no free slots".into());
        assert_eq!(Failure::of(&slots), Some(Failure::Temporary));
    }

    #[test]
    fn recoveries_escalate_and_stay_at_the_last_step() {
        let steps: Vec<Step> = (1..=6).map(Step::of).collect();
        assert_eq!(
            steps,
            [
                Step::Resubmit,
                Step::ResetState,
                Step::Relight,
                Step::ResetDevice,
                Step::ResetDevice,
                Step::ResetDevice
            ]
        );
        for step in [Step::Resubmit, Step::ResetState, Step::Relight, Step::ResetDevice] {
            assert_eq!(Step::of(step.recoveries()), step);
        }
    }

    #[test]
    fn a_cause_can_skip_the_gentle_steps_but_never_goes_back() {
        assert_eq!(Cause::StuckFlip.first_step(), Step::Resubmit);
        assert_eq!(Cause::Refused(Failure::Busy).first_step(), Step::ResetState);
        assert_eq!(Cause::Refused(Failure::Rejected).first_step(), Step::Relight);
        assert_eq!(Cause::Refused(Failure::TestFailed).first_step(), Step::Relight);
        assert_eq!(Cause::Requested.first_step(), Step::Relight);
        assert_eq!(Cause::Device.first_step(), Step::ResetDevice);
        // The step taken is the harder of the cause's floor and the
        // escalation so far, the way the backend works it out.
        let taken = |recoveries: u8, cause: Cause| {
            Step::of(recoveries.saturating_add(1).max(cause.first_step().recoveries()))
        };
        assert_eq!(taken(0, Cause::StuckFlip), Step::Resubmit);
        assert_eq!(taken(2, Cause::StuckFlip), Step::Relight);
        assert_eq!(taken(0, Cause::Requested), Step::Relight);
        assert_eq!(taken(3, Cause::Requested), Step::ResetDevice);
    }

    #[test]
    fn only_a_display_that_keeps_failing_waits_between_recoveries() {
        assert_eq!(backoff(1), Duration::ZERO);
        assert_eq!(backoff(4), Duration::ZERO);
        assert_eq!(backoff(5), Duration::from_secs(5));
        assert_eq!(backoff(8), Duration::from_secs(60));
        assert_eq!(backoff(u8::MAX), Duration::from_secs(60));
    }

    #[test]
    fn refused_frames_are_retried_ever_more_slowly() {
        let frame = Duration::from_micros(4167);
        assert_eq!(retry_delay(1, frame), frame);
        assert_eq!(retry_delay(3, frame), frame);
        assert_eq!(retry_delay(4, frame), Duration::from_millis(50));
        assert_eq!(retry_delay(11, frame), Duration::from_millis(250));
        // A slow display never retries faster than its own refresh.
        let slow = Duration::from_millis(400);
        assert_eq!(retry_delay(11, slow), slow);
    }
}
