//! A pulse on the compositor's own event loop, taken from outside it.
//!
//! Everything else that watches for a stopped display runs *on* that loop, so
//! everything else stops when it does. One thread does not. It sleeps, wakes,
//! and reads a number the loop writes every turn; if the number has stopped
//! moving then nothing in the compositor is running -- not input, not
//! repaints, not clients -- and every display is frozen at once. That is the
//! one failure the per-output watchdog can never report, because it would
//! have to run to report it.
//!
//! The loop is never quiet for long even with nothing to do: the output
//! watchdog's timer wakes it twice a second, so silence really is silence.
//!
//! Mostly this only writes the truth into the journal. Killing something to
//! break a stall would as often turn a hitch into a crash -- a GPU reset or a
//! slow modeset can hold the loop for seconds and then let go. A loop that
//! stays parked in a futex does not let go: that is a lock nobody will
//! release, and on 2026-09-12 it held both displays for over a minute until
//! the machine was switched off. So past `GIVE_UP` the watcher aborts the
//! compositor. The session ends and the greeter comes back instead of a hard
//! reboot, and systemd-coredump keeps every thread's stack, which says which
//! lock it was -- the one thing this module cannot read from `/proc`.
//!
//! What it buys otherwise is that a whole-desktop freeze stops being a
//! mystery: the line it writes says how long the loop has been gone *and where it is*, because
//! the kernel will tell any thread of a process where its siblings are
//! stopped -- `/proc/<tid>/syscall` and `/proc/<tid>/wchan`, no privilege
//! needed. Blocked in a read on a socket is a different bug from blocked in
//! an ioctl on the card, and the difference is otherwise hours. The worst
//! stall of the session is reported by `get_graphics` as `loop_stall_ms`.
//!
//! Two seconds is a freeze. What a desktop is actually judged on is much
//! smaller than that: a turn of the loop that burns four milliseconds has
//! already spent a whole frame's budget on a 240 Hz panel, and nobody has
//! frozen, nothing has been reported, and the frame is gone all the same.
//! So the heartbeat measures itself as well. Every turn is charged the
//! processor time it used -- not the wall time, which is mostly the loop
//! asleep with nothing to do -- and the turns that cost more than a frame
//! are counted. That is the difference between "the desktop feels heavy"
//! and a number that says the compositor's own thread is the reason.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tracing::{error, warn};

/// How long the loop has to be silent before it is worth saying so. The
/// output watchdog ticks every 500 ms and a page flip is given a whole
/// second, so anything under this can still be an honest modeset.
const STALL: Duration = Duration::from_secs(2);

/// How often the watcher looks. Fine enough to time a stall usefully,
/// coarse enough to cost nothing.
const TICK: Duration = Duration::from_millis(250);

/// How long the loop may sit in a futex before it is taken to be deadlocked
/// and the compositor is aborted. Nothing honest waits on a lock this long.
const GIVE_UP: Duration = Duration::from_secs(10);

/// How long any other stall is allowed before the same happens. Longer,
/// because a stall in the card's ioctl can be a GPU reset that recovers --
/// but past this a person has already reached for the power button.
const GIVE_UP_ANYWHERE: Duration = Duration::from_secs(30);

/// A turn of the loop that spends more processor time than this has cost a
/// frame on the fastest panel MindOS is likely to be driving. The loop turns
/// many times per frame when anything is happening, so one turn reaching a
/// whole frame's budget is not a busy desktop, it is something taking far
/// longer than it should.
const HITCH: Duration = Duration::from_micros(4_166);

/// The loop's heartbeat, shared with the thread that watches it.
#[derive(Clone, Debug, Default)]
pub struct LoopWatch {
    inner: Arc<Beats>,
}

/// What the loop writes and the watcher reads. One allocation, so a clone
/// of the watch is a pointer copy and the loop is never cloning per turn.
#[derive(Debug)]
struct Beats {
    origin: Instant,
    /// Microseconds since `origin` at the last turn of the loop.
    beat: AtomicU64,
    /// The longest silence seen, in microseconds; zero until one is long
    /// enough to count as a stall.
    worst: AtomicU64,
    /// Processor time this thread had used at the last turn, in
    /// microseconds; zero before the first turn.
    cpu: AtomicU64,
    /// Turns of the loop measured, and the processor time they used
    /// between them.
    turns: AtomicU64,
    busy: AtomicU64,
    /// The most processor time a single turn has used.
    worst_busy: AtomicU64,
    /// The longest a turn has taken by the clock on the wall, which at idle
    /// is the loop asleep and not the loop stuck: see `worst_gap`.
    worst_gap: AtomicU64,
    /// Turns that used more than `HITCH`.
    hitches: AtomicU64,
    /// What the next hitch has to beat before it is worth a line in the
    /// journal, in microseconds.
    announced: AtomicU64,
}

impl Default for Beats {
    fn default() -> Self {
        Beats {
            origin: Instant::now(),
            beat: AtomicU64::new(0),
            worst: AtomicU64::new(0),
            cpu: AtomicU64::new(0),
            turns: AtomicU64::new(0),
            busy: AtomicU64::new(0),
            worst_busy: AtomicU64::new(0),
            worst_gap: AtomicU64::new(0),
            hitches: AtomicU64::new(0),
            announced: AtomicU64::new(HITCH.as_micros() as u64),
        }
    }
}

impl LoopWatch {
    /// Called once per turn of the event loop, from the loop.
    ///
    /// Everything here is one clock read, one syscall and a handful of
    /// uncontended atomics, on a thread that turns a few thousand times a
    /// second at its busiest: well under a millisecond of processor time per
    /// second of desktop, to find the four-millisecond turns that are the
    /// whole reason anyone notices the compositor at all.
    pub fn beat(&self) {
        let now = self.inner.origin.elapsed().as_micros() as u64;
        let cpu = thread_cpu_micros();
        let before = self.inner.beat.swap(now, Ordering::Relaxed);
        let before_cpu = self.inner.cpu.swap(cpu, Ordering::Relaxed);
        // Nothing to measure against on the first turn, and measuring
        // against zero would charge the whole of start-up to it.
        if before_cpu == 0 {
            return;
        }
        let busy = cpu.saturating_sub(before_cpu);
        self.inner.turns.fetch_add(1, Ordering::Relaxed);
        self.inner.busy.fetch_add(busy, Ordering::Relaxed);
        self.inner.worst_busy.fetch_max(busy, Ordering::Relaxed);
        self.inner.worst_gap.fetch_max(now.saturating_sub(before), Ordering::Relaxed);
        if busy >= HITCH.as_micros() as u64 {
            self.inner.hitches.fetch_add(1, Ordering::Relaxed);
            self.say_if_worst(busy);
        }
    }

    /// Put a hitch in the journal, if it is the worst so far by a clear
    /// margin.
    ///
    /// Not every hitch: a compositor having a bad second could have hundreds
    /// of them, and then the journal is the problem. Reporting only what
    /// beats the last report, and only at double, leaves a handful of lines
    /// in a session and always includes the worst one. The timestamp is what
    /// this is for -- the count in `get_graphics` says a desktop stutters,
    /// and a line in the journal says what else was happening when it did.
    fn say_if_worst(&self, busy: u64) {
        // Only the loop calls `beat`, so this is read and written by one
        // thread; the atomic is for the watcher, which never touches it.
        if busy < self.inner.announced.load(Ordering::Relaxed) {
            return;
        }
        self.inner.announced.store(busy.saturating_mul(2), Ordering::Relaxed);
        warn!(
            took_us = busy,
            "a turn of the event loop cost more than a frame: every display waited for it"
        );
    }

    /// How long since the loop last turned.
    pub fn silent_for(&self) -> Duration {
        let beat = Duration::from_micros(self.inner.beat.load(Ordering::Relaxed));
        self.inner.origin.elapsed().saturating_sub(beat)
    }

    /// The longest stall of the session; zero if there has never been one.
    pub fn worst_stall(&self) -> Duration {
        Duration::from_micros(self.inner.worst.load(Ordering::Relaxed))
    }

    /// The most processor time one turn of the loop has ever used.
    ///
    /// This is the number to look at when frames are late and no display is
    /// frozen. Every client's commit, every input event and every frame is
    /// drawn on this one thread, so a turn that takes long enough is a
    /// stutter on every display at once, whoever caused it.
    pub fn worst_turn(&self) -> Duration {
        Duration::from_micros(self.inner.worst_busy.load(Ordering::Relaxed))
    }

    /// The longest a turn has taken by the wall clock.
    ///
    /// Usually the loop asleep with nothing to do, which is not a fault and
    /// is why the hitch counting uses processor time instead. It is worth
    /// reporting anyway because the output watchdog's timer wakes the loop
    /// twice a second whatever else is happening: a gap much over half a
    /// second is the loop stuck in a call, not the loop resting.
    pub fn worst_gap(&self) -> Duration {
        Duration::from_micros(self.inner.worst_gap.load(Ordering::Relaxed))
    }

    /// Turns that used more processor time than a frame is worth, and turns
    /// in total.
    pub fn hitches(&self) -> (u64, u64) {
        (
            self.inner.hitches.load(Ordering::Relaxed),
            self.inner.turns.load(Ordering::Relaxed),
        )
    }

    /// What share of the session the loop has spent on the processor, as a
    /// percentage. The desktop is idle most of the time, so this is small
    /// whenever the compositor is behaving and worth chasing when it is not.
    pub fn busy_percent(&self) -> f64 {
        let elapsed = self.inner.origin.elapsed().as_micros() as u64;
        if elapsed == 0 {
            return 0.0;
        }
        self.inner.busy.load(Ordering::Relaxed) as f64 * 100.0 / elapsed as f64
    }

    /// Start watching, from the thread that runs the loop. The watcher ends
    /// when the loop's own watch is dropped, which in the compositor is when
    /// the process ends and in the tests is when their compositor does.
    pub fn watch(&self) {
        let watch = self.clone();
        // Whoever calls this is the loop, and it is that thread the kernel
        // will be asked about later.
        let tid = unsafe { libc::gettid() };
        if let Err(err) = std::thread::Builder::new()
            .name("mindwm-loop-watch".into())
            .spawn(move || watch.run(tid))
        {
            warn!(%err, "cannot watch the event loop for stalls");
        }
    }

    fn run(self, tid: libc::pid_t) {
        // The threshold the next complaint has to beat, so one long stall
        // says so as it grows instead of once every tick.
        let mut announced: Option<Duration> = None;
        loop {
            std::thread::sleep(TICK);
            if Arc::strong_count(&self.inner) == 1 {
                // Nothing is left to beat: the loop is over, not stuck.
                return;
            }
            let silent = self.silent_for();
            if silent >= STALL {
                self.inner.worst.fetch_max(silent.as_micros() as u64, Ordering::Relaxed);
                let stopped_at = where_is(tid);
                if announced.is_none_or(|next| silent >= next) {
                    error!(
                        silent_ms = silent.as_millis() as u64,
                        %stopped_at,
                        "the event loop has not turned: every display is frozen until it does"
                    );
                    announced = Some(silent * 2);
                }
                if gives_up(silent, &stopped_at) {
                    error!(
                        silent_ms = silent.as_millis() as u64,
                        %stopped_at,
                        "the event loop is deadlocked: aborting so the session can end and the core names the lock"
                    );
                    // Let the journal have the line before the process goes.
                    std::thread::sleep(Duration::from_millis(200));
                    std::process::abort();
                }
            } else if let Some(announced) = announced.take() {
                warn!(
                    stalled_ms = (announced / 2).as_millis() as u64,
                    "the event loop is turning again"
                );
            }
        }
    }
}

/// Whether a stall this long, stopped where `where_is` says, is past saving.
fn gives_up(silent: Duration, stopped_at: &str) -> bool {
    silent >= GIVE_UP_ANYWHERE || (silent >= GIVE_UP && stopped_at.starts_with("futex"))
}

/// Processor time the calling thread has used, in microseconds.
///
/// Deliberately not the wall clock: a loop that sleeps for a second because
/// the desktop is idle has done nothing wrong, and a loop that spends a
/// tenth of a second computing has cost every display twenty frames. Only
/// this one tells them apart.
///
/// Unlike the monotonic clock this is a real syscall rather than a vDSO
/// read, at some tens of nanoseconds. A failure -- which cannot happen for
/// the calling thread's own clock -- reports zero, which the caller reads as
/// "nothing to compare against" and skips, so a broken clock loses the
/// measurement rather than inventing one.
fn thread_cpu_micros() -> u64 {
    let mut spec = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    if unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut spec) } != 0 {
        return 0;
    }
    (spec.tv_sec as u64).saturating_mul(1_000_000) + (spec.tv_nsec as u64) / 1_000
}

/// Where a thread of this process is stopped, as the kernel sees it.
fn where_is(tid: libc::pid_t) -> String {
    let read = |what: &str| std::fs::read_to_string(format!("/proc/self/task/{tid}/{what}")).unwrap_or_default();
    let (call, fd) = describe(&read("syscall"), &read("wchan"));
    let on = fd
        .and_then(|fd| std::fs::read_link(format!("/proc/self/fd/{fd}")).ok())
        .map(|path| format!(" on {}", path.display()))
        .unwrap_or_default();
    format!("{call}{on}")
}

/// `/proc/<tid>/syscall` is the syscall number then its arguments, the
/// number in decimal and the arguments in hex, or the word `running` when
/// the thread is not in a syscall at all. `/proc/<tid>/wchan` is the kernel
/// function it is sleeping in. Returns what to print, and the file
/// descriptor the call is on when it has one worth naming.
fn describe(syscall: &str, wchan: &str) -> (String, Option<i64>) {
    let mut fields = syscall.split_whitespace();
    let nr = fields.next().unwrap_or_default();
    let number = nr.parse::<i64>().ok();
    let call = match (number, nr) {
        (Some(nr), _) => syscall_name(nr).map(str::to_string).unwrap_or_else(|| format!("syscall {nr}")),
        (None, "") => "somewhere unreadable".to_string(),
        (None, other) => other.to_string(),
    };
    // Only the calls that take a descriptor first, or the number is an
    // address and the link lookup is nonsense.
    let fd = number
        .filter(|nr| matches!(nr, 0 | 1 | 16 | 46 | 47 | 232 | 281 | 441))
        .and_then(|_| fields.next())
        .and_then(|arg| i64::from_str_radix(arg.trim_start_matches("0x"), 16).ok())
        .filter(|fd| (0..4096).contains(fd));
    let wchan = wchan.trim();
    let waiting = if wchan.is_empty() || wchan == "0" {
        String::new()
    } else {
        format!(" waiting in {wchan}")
    };
    (format!("{call}{waiting}"), fd)
}

/// The x86_64 numbers worth naming. Anything else is printed as a number,
/// which `ausyscall` translates.
#[cfg(target_arch = "x86_64")]
fn syscall_name(nr: i64) -> Option<&'static str> {
    Some(match nr {
        0 => "read",
        1 => "write",
        7 => "poll",
        16 => "ioctl",
        23 => "select",
        46 => "sendmsg",
        47 => "recvmsg",
        202 => "futex",
        232 => "epoll_wait",
        271 => "ppoll",
        281 => "epoll_pwait",
        441 => "epoll_pwait2",
        _ => return None,
    })
}

#[cfg(not(target_arch = "x86_64"))]
fn syscall_name(_nr: i64) -> Option<&'static str> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silence_is_measured_from_the_last_turn() {
        let watch = LoopWatch::default();
        // Nothing has beaten yet, so the whole life of the watch is silence.
        assert!(watch.silent_for() < Duration::from_secs(1));
        assert_eq!(watch.worst_stall(), Duration::ZERO);
        std::thread::sleep(Duration::from_millis(20));
        let before = watch.silent_for();
        watch.beat();
        assert!(watch.silent_for() < before);
        // A clone shares the heartbeat: that is the whole point of it.
        let other = watch.clone();
        std::thread::sleep(Duration::from_millis(10));
        let stale = other.silent_for();
        watch.beat();
        assert!(other.silent_for() < stale);
    }

    /// The distinction the whole per-turn measurement rests on: a turn that
    /// sleeps is the desktop being idle, and a turn that computes for longer
    /// than a frame is a stutter on every display. Only one of them counts.
    #[test]
    fn sleeping_is_not_a_hitch_and_computing_is() {
        let watch = LoopWatch::default();
        watch.beat(); // The first has nothing to measure against.

        // A quiet loop: a long turn by the wall clock, no processor time.
        std::thread::sleep(HITCH * 8);
        watch.beat();
        let (hitches, turns) = watch.hitches();
        assert_eq!(hitches, 0, "a loop with nothing to do is not stuttering");
        assert_eq!(turns, 1);
        assert!(watch.worst_gap() >= HITCH * 8, "the wall clock still saw it");
        assert!(watch.worst_turn() < HITCH, "but it cost no processor time");

        // A turn that actually works for longer than a frame is worth.
        let until = Instant::now() + HITCH * 2;
        let mut spun = 0u64;
        while Instant::now() < until {
            spun = spun.wrapping_add(1);
        }
        assert!(spun > 0);
        watch.beat();
        let (hitches, turns) = watch.hitches();
        assert_eq!(hitches, 1, "a turn that ate a frame is counted");
        assert_eq!(turns, 2);
        assert!(watch.worst_turn() >= HITCH, "{:?}", watch.worst_turn());
        assert!(watch.busy_percent() > 0.0);
        // Still nothing a watcher would call a stall: those are seconds.
        assert_eq!(watch.worst_stall(), Duration::ZERO);
    }

    /// The clock the hitch counting is built on has to actually move, and
    /// has to stay put while the thread is asleep.
    #[test]
    fn processor_time_counts_work_and_not_waiting() {
        let start = thread_cpu_micros();
        assert!(start > 0, "this thread has run at least a little");
        std::thread::sleep(Duration::from_millis(50));
        let slept = thread_cpu_micros();
        assert!(
            slept - start < 10_000,
            "sleeping charged {} us of processor time",
            slept - start
        );
        let until = Instant::now() + Duration::from_millis(20);
        while Instant::now() < until {}
        assert!(thread_cpu_micros() - slept >= 10_000, "working charged nothing");
    }

    #[test]
    fn a_stall_says_which_syscall_it_is_in() {
        // Blocked reading a socket: XWayland, a client, the IPC.
        let (what, fd) = describe("0 0x2a 0x7ffd 0x1000 0x0 0x0 0x0", "sock_wait_data");
        assert_eq!(what, "read waiting in sock_wait_data");
        assert_eq!(fd, Some(42));
        // Blocked on the card.
        let (what, fd) = describe("16 0x9 0xc0406469", "drm_wait_vblank");
        assert_eq!(what, "ioctl waiting in drm_wait_vblank");
        assert_eq!(fd, Some(9));
        // Asleep waiting for events: the ordinary idle loop, no descriptor
        // worth chasing but still named.
        let (what, _) = describe("281 0x5 0x0", "do_epoll_wait");
        assert_eq!(what, "epoll_pwait waiting in do_epoll_wait");
        // A call with no descriptor argument does not invent one out of a
        // pointer.
        assert_eq!(describe("202 0x7ffd0000", "futex_wait").1, None);
        // Not in a syscall at all, and nothing readable at all.
        assert_eq!(describe("running", "0").0, "running");
        assert_eq!(describe("", "").0, "somewhere unreadable");
        // An unknown number is still worth printing.
        assert_eq!(describe("9999", "").0, "syscall 9999");
    }

    /// A lock held for ten seconds is a deadlock; ten seconds on the card
    /// can still be a GPU reset, but thirty seconds of anything is not.
    #[test]
    fn a_deadlock_is_given_up_on_and_a_slow_card_is_waited_for() {
        let futex = "futex waiting in futex_do_wait";
        let ioctl = "ioctl waiting in drm_wait_vblank on /dev/dri/card1";
        assert!(!gives_up(Duration::from_secs(9), futex));
        assert!(gives_up(Duration::from_secs(10), futex));
        assert!(!gives_up(Duration::from_secs(10), ioctl));
        assert!(!gives_up(Duration::from_secs(29), ioctl));
        assert!(gives_up(Duration::from_secs(30), ioctl));
    }

    /// The parsing above is only worth anything if this kernel really does
    /// answer for a thread that is stuck, so ask it about one that is.
    #[test]
    fn the_kernel_says_where_a_stopped_thread_is() {
        use std::io::Read;
        use std::os::unix::io::FromRawFd;

        let mut ends = [0 as libc::c_int; 2];
        assert_eq!(unsafe { libc::pipe(ends.as_mut_ptr()) }, 0);
        let [read_end, write_end] = ends;
        let (tx, rx) = std::sync::mpsc::channel();
        let stuck = std::thread::spawn(move || {
            tx.send(unsafe { libc::gettid() }).unwrap();
            // Blocks until the test below writes the byte.
            let mut file = unsafe { std::fs::File::from_raw_fd(read_end) };
            let _ = file.read(&mut [0u8; 1]);
        });
        let tid = rx.recv().unwrap();

        let mut what = String::new();
        for _ in 0..200 {
            what = where_is(tid);
            if what.starts_with("read") {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(what.starts_with("read"), "{what}");
        assert!(what.contains("waiting in "), "{what}");
        assert!(what.contains("pipe:"), "{what}");

        assert_eq!(unsafe { libc::write(write_end, b"x".as_ptr().cast(), 1) }, 1);
        stuck.join().unwrap();
        unsafe { libc::close(write_end) };
    }
}
