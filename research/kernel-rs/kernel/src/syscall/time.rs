//! Time and timer system calls.

use super::SysResult;
use crate::arch::x86_64::tsc::uptime_ns;
use crate::mm::errno::*;
use crate::mm::user::{read_user, write_user};
use crate::proc::process::ITimer;

pub const CLOCK_REALTIME: i32 = 0;
pub const CLOCK_MONOTONIC: i32 = 1;
pub const CLOCK_PROCESS_CPUTIME_ID: i32 = 2;
pub const CLOCK_THREAD_CPUTIME_ID: i32 = 3;
pub const CLOCK_MONOTONIC_RAW: i32 = 4;
pub const CLOCK_REALTIME_COARSE: i32 = 5;
pub const CLOCK_MONOTONIC_COARSE: i32 = 6;
pub const CLOCK_BOOTTIME: i32 = 7;

pub fn write_timespec(addr: usize, ns: u64) -> Result<()> {
    write_user::<[i64; 2]>(addr, [(ns / 1_000_000_000) as i64, (ns % 1_000_000_000) as i64])
}

pub fn read_timespec(addr: usize) -> Result<u64> {
    let [s, ns]: [i64; 2] = read_user(addr)?;
    if s < 0 || ns < 0 || ns >= 1_000_000_000 {
        return Err(EINVAL);
    }
    Ok(s as u64 * 1_000_000_000 + ns as u64)
}

pub fn clock_now(clk: i32) -> Result<u64> {
    match clk {
        CLOCK_REALTIME | CLOCK_REALTIME_COARSE => Ok(crate::dev::rtc::wall_time_ns()),
        CLOCK_MONOTONIC | CLOCK_MONOTONIC_RAW | CLOCK_MONOTONIC_COARSE | CLOCK_BOOTTIME => Ok(uptime_ns()),
        CLOCK_PROCESS_CPUTIME_ID => {
            let p = &crate::sched::current_ref().proc;
            Ok(p.threads.lock().iter().map(|t| t.cpu_time_ns.load(core::sync::atomic::Ordering::Relaxed)).sum())
        }
        CLOCK_THREAD_CPUTIME_ID => Ok(crate::sched::current_ref().cpu_time_ns.load(core::sync::atomic::Ordering::Relaxed)),
        _ => Err(EINVAL),
    }
}

pub fn clock_gettime(clk: i32, ts: usize) -> SysResult {
    let now = clock_now(clk)?;
    write_timespec(ts, now)?;
    Ok(0)
}

pub fn clock_settime(clk: i32, ts: usize) -> SysResult {
    if clk != CLOCK_REALTIME {
        return Err(EINVAL);
    }
    let ns = read_timespec(ts)?;
    crate::dev::rtc::set_wall_time_ns(ns);
    Ok(0)
}

pub fn clock_getres(clk: i32, ts: usize) -> SysResult {
    clock_now(clk)?;
    if ts != 0 {
        write_timespec(ts, 1)?;
    }
    Ok(0)
}

pub fn gettimeofday(tv: usize, _tz: usize) -> SysResult {
    if tv != 0 {
        let now = crate::dev::rtc::wall_time_ns();
        write_user::<[i64; 2]>(tv, [(now / 1_000_000_000) as i64, ((now % 1_000_000_000) / 1000) as i64])?;
    }
    Ok(0)
}

pub fn time(tloc: usize) -> SysResult {
    let now = crate::dev::rtc::wall_time_ns() / 1_000_000_000;
    if tloc != 0 {
        write_user::<i64>(tloc, now as i64)?;
    }
    Ok(now)
}

fn sleep_until(deadline: u64, rem: usize, relative_start: Option<u64>) -> SysResult {
    match crate::sched::wait::sleep_until(deadline) {
        Ok(()) => Ok(0),
        Err(_) => {
            if rem != 0 {
                if let Some(_) = relative_start {
                    let now = uptime_ns();
                    let left = deadline.saturating_sub(now);
                    write_timespec(rem, left)?;
                }
            }
            Err(EINTR)
        }
    }
}

pub fn nanosleep(req: usize, rem: usize) -> SysResult {
    let ns = read_timespec(req)?;
    let start = uptime_ns();
    sleep_until(start + ns, rem, Some(start))
}

pub fn clock_nanosleep(clk: i32, flags: i32, req: usize, rem: usize) -> SysResult {
    let ns = read_timespec(req)?;
    match clk {
        CLOCK_REALTIME | CLOCK_MONOTONIC | CLOCK_BOOTTIME | CLOCK_MONOTONIC_RAW => {}
        _ => return Err(EINVAL),
    }
    if flags & 1 != 0 {
        // TIMER_ABSTIME
        let now = clock_now(clk)?;
        let deadline = uptime_ns() + ns.saturating_sub(now);
        sleep_until(deadline, 0, None)
    } else {
        let start = uptime_ns();
        sleep_until(start + ns, rem, Some(start))
    }
}

pub fn times(buf: usize) -> SysResult {
    let ticks = uptime_ns() / 10_000_000; // 100 Hz clock ticks
    if buf != 0 {
        let p = &crate::sched::current_ref().proc;
        let cpu: u64 = p.threads.lock().iter().map(|t| t.cpu_time_ns.load(core::sync::atomic::Ordering::Relaxed)).sum();
        let ut = (cpu / 10_000_000) as i64;
        write_user::<[i64; 4]>(buf, [ut, 0, 0, 0])?;
    }
    Ok(ticks)
}

fn read_timeval(addr: usize) -> Result<u64> {
    let [s, us]: [i64; 2] = read_user(addr)?;
    if s < 0 || us < 0 || us >= 1_000_000 {
        return Err(EINVAL);
    }
    Ok(s as u64 * 1_000_000_000 + us as u64 * 1000)
}

fn write_itimerval(addr: usize, interval_ns: u64, value_ns: u64) -> Result<()> {
    let tv = |ns: u64| [(ns / 1_000_000_000) as i64, ((ns % 1_000_000_000) / 1000) as i64];
    let i = tv(interval_ns);
    let v = tv(value_ns);
    write_user::<[i64; 4]>(addr, [i[0], i[1], v[0], v[1]])
}

pub fn getitimer(which: i32, cur: usize) -> SysResult {
    if which != 0 {
        return Err(EINVAL);
    }
    let it = *crate::sched::current_ref().proc.itimer_real.lock();
    let left = if it.next_ns == 0 { 0 } else { it.next_ns.saturating_sub(uptime_ns()) };
    write_itimerval(cur, it.interval_ns, left)?;
    Ok(0)
}

pub fn setitimer(which: i32, new: usize, old: usize) -> SysResult {
    if which != 0 {
        return if which == 1 || which == 2 { Ok(0) } else { Err(EINVAL) };
    }
    let p = &crate::sched::current_ref().proc;
    let prev = *p.itimer_real.lock();
    if old != 0 {
        let left = if prev.next_ns == 0 { 0 } else { prev.next_ns.saturating_sub(uptime_ns()) };
        write_itimerval(old, prev.interval_ns, left)?;
    }
    if new != 0 {
        let interval = read_timeval(new)?;
        let value = read_timeval(new + 16)?;
        let mut it = p.itimer_real.lock();
        *it = ITimer { next_ns: if value == 0 { 0 } else { uptime_ns() + value }, interval_ns: interval };
    }
    Ok(0)
}

pub fn alarm(secs: u32) -> SysResult {
    let p = &crate::sched::current_ref().proc;
    let mut it = p.itimer_real.lock();
    let left = if it.next_ns == 0 { 0 } else { (it.next_ns.saturating_sub(uptime_ns()) + 999_999_999) / 1_000_000_000 };
    *it = ITimer { next_ns: if secs == 0 { 0 } else { uptime_ns() + secs as u64 * 1_000_000_000 }, interval_ns: 0 };
    Ok(left)
}
