//! poll/select/epoll, eventfd and timerfd.

use super::SysResult;
use crate::fs::vfs::{File, FileOps, POLLERR, POLLHUP, POLLIN, POLLNVAL, POLLOUT};
use crate::mm::errno::*;
use crate::mm::user::{copy_from_user, copy_to_user, read_user, write_user};
use crate::sched::wait::{PollTable, WaitQueue};
use crate::sync::SpinLock;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

#[repr(C)]
#[derive(Clone, Copy)]
struct PollFd {
    fd: i32,
    events: i16,
    revents: i16,
}

fn deadline_from_ms(ms: i64) -> u64 {
    if ms < 0 {
        0
    } else {
        crate::arch::x86_64::tsc::uptime_ns() + ms as u64 * 1_000_000
    }
}

fn do_poll(fds: &mut [PollFd], deadline: u64) -> Result<usize> {
    let table = super::fs::files();
    let files: Vec<Option<Arc<File>>> = fds.iter().map(|p| if p.fd < 0 { None } else { table.get(p.fd).ok() }).collect();
    let mut pt = PollTable::new();
    let mut registered = false;
    loop {
        if !registered {
            pt.prepare();
        }
        let mut ready = 0;
        for (i, p) in fds.iter_mut().enumerate() {
            p.revents = 0;
            if p.fd < 0 {
                continue;
            }
            match &files[i] {
                None => {
                    p.revents = POLLNVAL as i16;
                    ready += 1;
                }
                Some(f) => {
                    if !registered {
                        if let Some(wq) = f.ops.poll_wait() {
                            pt.add(wq);
                        }
                    }
                    let mask = f.ops.poll(f);
                    let want = (p.events as u16 as u32) | POLLERR | POLLHUP;
                    let r = mask & want;
                    if r != 0 {
                        p.revents = r as i16;
                        ready += 1;
                    }
                }
            }
        }
        registered = true;
        if ready > 0 {
            pt.cancel();
            return Ok(ready);
        }
        if deadline == u64::MAX {
            pt.cancel();
            return Ok(0);
        }
        match pt.block(deadline) {
            Ok(true) => {
                pt.prepare();
                continue;
            }
            Ok(false) => return Ok(0),
            Err(_) => return Err(EINTR),
        }
    }
}

pub fn poll(fds: usize, nfds: usize, timeout_ms: i64) -> SysResult {
    if nfds > 4096 {
        return Err(EINVAL);
    }
    let mut v = alloc::vec![PollFd { fd: -1, events: 0, revents: 0 }; nfds];
    let bytes = unsafe { core::slice::from_raw_parts_mut(v.as_mut_ptr() as *mut u8, nfds * 8) };
    copy_from_user(bytes, fds)?;
    let deadline = if timeout_ms == 0 { u64::MAX } else { deadline_from_ms(timeout_ms) };
    let n = do_poll(&mut v, deadline)?;
    let bytes = unsafe { core::slice::from_raw_parts(v.as_ptr() as *const u8, nfds * 8) };
    copy_to_user(fds, bytes)?;
    Ok(n as u64)
}

pub fn ppoll(fds: usize, nfds: usize, tmo: usize, sigmask: usize, sigsetsize: usize) -> SysResult {
    let timeout_ms = if tmo == 0 {
        -1
    } else {
        let ns = super::time::read_timespec(tmo)?;
        if ns == 0 {
            0
        } else {
            ((ns + 999_999) / 1_000_000) as i64
        }
    };
    let saved = set_temp_sigmask(sigmask, sigsetsize)?;
    let r = poll(fds, nfds, timeout_ms);
    restore_sigmask(saved);
    r
}

fn set_temp_sigmask(sigmask: usize, sigsetsize: usize) -> Result<Option<u64>> {
    if sigmask == 0 {
        return Ok(None);
    }
    if sigsetsize != 8 {
        return Err(EINVAL);
    }
    let m: u64 = read_user(sigmask)?;
    let t = crate::sched::current_ref();
    let mut s = t.sig.lock();
    let old = s.mask;
    s.mask = m & !((1 << 8) | (1 << 18));
    Ok(Some(old))
}

fn restore_sigmask(saved: Option<u64>) {
    if let Some(m) = saved {
        let t = crate::sched::current_ref();
        // if a signal is now deliverable, keep the temporary mask until delivery
        if t.has_pending_signal() {
            t.saved_mask.store(m, Ordering::Relaxed);
        } else {
            t.sig.lock().mask = m;
        }
    }
}

fn read_fdset(addr: usize, n: usize) -> Result<Vec<u64>> {
    let words = (n + 63) / 64;
    let mut v = alloc::vec![0u64; words];
    if addr != 0 && words > 0 {
        let bytes = unsafe { core::slice::from_raw_parts_mut(v.as_mut_ptr() as *mut u8, words * 8) };
        copy_from_user(bytes, addr)?;
    }
    Ok(v)
}

fn write_fdset(addr: usize, v: &[u64]) -> Result<()> {
    if addr != 0 && !v.is_empty() {
        let bytes = unsafe { core::slice::from_raw_parts(v.as_ptr() as *const u8, v.len() * 8) };
        copy_to_user(addr, bytes)?;
    }
    Ok(())
}

fn do_select(n: i32, r: usize, w: usize, e: usize, deadline: u64) -> SysResult {
    if n < 0 || n > 4096 {
        return Err(EINVAL);
    }
    let n = n as usize;
    let rs = read_fdset(r, n)?;
    let ws = read_fdset(w, n)?;
    let es = read_fdset(e, n)?;
    let mut fds = Vec::new();
    for fd in 0..n {
        let bit = 1u64 << (fd % 64);
        let mut ev = 0i16;
        if rs.get(fd / 64).map(|x| x & bit != 0).unwrap_or(false) {
            ev |= POLLIN as i16;
        }
        if ws.get(fd / 64).map(|x| x & bit != 0).unwrap_or(false) {
            ev |= POLLOUT as i16;
        }
        if es.get(fd / 64).map(|x| x & bit != 0).unwrap_or(false) {
            ev |= 2; // POLLPRI
        }
        if ev != 0 {
            fds.push(PollFd { fd: fd as i32, events: ev, revents: 0 });
        }
    }
    let cnt = do_poll(&mut fds, deadline)?;
    let mut ro = alloc::vec![0u64; rs.len()];
    let mut wo = alloc::vec![0u64; ws.len()];
    let mut eo = alloc::vec![0u64; es.len()];
    let mut total = 0;
    for p in &fds {
        if p.revents & POLLNVAL as i16 != 0 {
            return Err(EBADF);
        }
        let fd = p.fd as usize;
        let bit = 1u64 << (fd % 64);
        let rv = p.revents as u16 as u32;
        if p.events & POLLIN as i16 != 0 && rv & (POLLIN | POLLHUP | POLLERR) != 0 {
            ro[fd / 64] |= bit;
            total += 1;
        }
        if p.events & POLLOUT as i16 != 0 && rv & (POLLOUT | POLLERR) != 0 {
            wo[fd / 64] |= bit;
            total += 1;
        }
    }
    let _ = cnt;
    write_fdset(r, &ro)?;
    write_fdset(w, &wo)?;
    write_fdset(e, &eo)?;
    Ok(total as u64)
}

pub fn select(n: i32, r: usize, w: usize, e: usize, timeout: usize, _pselect: bool) -> SysResult {
    let deadline = if timeout == 0 {
        0
    } else {
        let [s, us]: [i64; 2] = read_user(timeout)?;
        if s < 0 || us < 0 {
            return Err(EINVAL);
        }
        let ns = s as u64 * 1_000_000_000 + us as u64 * 1000;
        if ns == 0 {
            u64::MAX
        } else {
            crate::arch::x86_64::tsc::uptime_ns() + ns
        }
    };
    let start = crate::arch::x86_64::tsc::uptime_ns();
    let res = do_select(n, r, w, e, deadline);
    if timeout != 0 && deadline != u64::MAX {
        // update the remaining time like Linux does
        let now = crate::arch::x86_64::tsc::uptime_ns();
        let left = deadline.saturating_sub(now.max(start));
        let _ = write_user::<[i64; 2]>(timeout, [(left / 1_000_000_000) as i64, ((left % 1_000_000_000) / 1000) as i64]);
    }
    res
}

pub fn pselect6(n: i32, r: usize, w: usize, e: usize, timeout: usize, sig: usize) -> SysResult {
    let deadline = if timeout == 0 {
        0
    } else {
        let ns = super::time::read_timespec(timeout)?;
        if ns == 0 {
            u64::MAX
        } else {
            crate::arch::x86_64::tsc::uptime_ns() + ns
        }
    };
    let saved = if sig != 0 {
        let [mask_ptr, size]: [u64; 2] = read_user(sig)?;
        set_temp_sigmask(mask_ptr as usize, size as usize)?
    } else {
        None
    };
    let res = do_select(n, r, w, e, deadline);
    restore_sigmask(saved);
    res
}

// ---- epoll ---------------------------------------------------------------------

pub const EPOLL_CTL_ADD: i32 = 1;
pub const EPOLL_CTL_DEL: i32 = 2;
pub const EPOLL_CTL_MOD: i32 = 3;
pub const EPOLLET: u32 = 1 << 31;
pub const EPOLLONESHOT: u32 = 1 << 30;

struct Interest {
    fd: i32,
    file: Arc<File>,
    events: u32,
    data: u64,
}

pub struct Epoll {
    interests: SpinLock<Vec<Interest>>,
    wq: WaitQueue,
}

pub fn epoll_create(_flags: u32) -> SysResult {
    let ep = Arc::new(Epoll { interests: SpinLock::new(Vec::new()), wq: WaitQueue::new() });
    let f = File::new(None, ep, crate::fs::vfs::O_RDWR, "anon_inode:[eventpoll]");
    Ok(super::fs::files().alloc(f, _flags & 0o2000000 != 0, 0)? as u64)
}

fn get_epoll(fd: i32) -> Result<(Arc<File>, Arc<Epoll>)> {
    let f = super::fs::files().get(fd)?;
    let ep = f.ops.clone();
    let ep = Arc::downcast::<Epoll>(ep.as_any_arc()).map_err(|_| EINVAL)?;
    Ok((f, ep))
}

pub fn epoll_ctl(epfd: i32, op: i32, fd: i32, event: usize) -> SysResult {
    let (_, ep) = get_epoll(epfd)?;
    let file = super::fs::files().get(fd)?;
    let (events, data) = if op != EPOLL_CTL_DEL {
        let mut b = [0u8; 12];
        copy_from_user(&mut b, event)?;
        (u32::from_le_bytes(b[0..4].try_into().unwrap()), u64::from_le_bytes(b[4..12].try_into().unwrap()))
    } else {
        (0, 0)
    };
    let mut ints = ep.interests.lock();
    let pos = ints.iter().position(|i| i.fd == fd && Arc::ptr_eq(&i.file, &file));
    match op {
        EPOLL_CTL_ADD => {
            if pos.is_some() {
                return Err(EEXIST);
            }
            if file.ops.as_any().downcast_ref::<Epoll>().map(|e| core::ptr::eq(e, &*ep)).unwrap_or(false) {
                return Err(EINVAL);
            }
            ints.push(Interest { fd, file, events: events | POLLERR | POLLHUP, data });
        }
        EPOLL_CTL_MOD => {
            let i = pos.ok_or(ENOENT)?;
            ints[i].events = events | POLLERR | POLLHUP;
            ints[i].data = data;
        }
        EPOLL_CTL_DEL => {
            let i = pos.ok_or(ENOENT)?;
            ints.remove(i);
        }
        _ => return Err(EINVAL),
    }
    drop(ints);
    ep.wq.wake_all();
    Ok(0)
}

impl Epoll {
    fn ready(&self) -> Vec<(u32, u64)> {
        let ints = self.interests.lock();
        let mut out = Vec::new();
        for i in ints.iter() {
            let m = i.file.ops.poll(&i.file) & (i.events & !(EPOLLET | EPOLLONESHOT));
            if m != 0 {
                out.push((m, i.data));
            }
        }
        out
    }
}

impl FileOps for Epoll {
    fn read(&self, _f: &File, _b: &mut [u8]) -> Result<usize> {
        Err(EINVAL)
    }
    fn write(&self, _f: &File, _b: &[u8]) -> Result<usize> {
        Err(EINVAL)
    }
    fn poll(&self, _f: &File) -> u32 {
        if self.ready().is_empty() {
            0
        } else {
            POLLIN
        }
    }
    fn poll_wait(&self) -> Option<&WaitQueue> {
        Some(&self.wq)
    }
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
}

pub fn epoll_wait(epfd: i32, events: usize, maxevents: i32, timeout_ms: i64) -> SysResult {
    if maxevents <= 0 {
        return Err(EINVAL);
    }
    let (_, ep) = get_epoll(epfd)?;
    let deadline = if timeout_ms == 0 { u64::MAX } else { deadline_from_ms(timeout_ms) };
    epoll_wait_inner(&ep, events, maxevents as usize, deadline)
}

pub fn epoll_pwait2(epfd: i32, events: usize, maxevents: i32, timeout: usize) -> SysResult {
    let ms = if timeout == 0 { -1 } else { ((super::time::read_timespec(timeout)? + 999_999) / 1_000_000) as i64 };
    epoll_wait(epfd, events, maxevents, ms)
}

fn epoll_wait_inner(ep: &Arc<Epoll>, events: usize, maxevents: usize, deadline: u64) -> SysResult {
    let mut pt = PollTable::new();
    loop {
        pt.prepare();
        // register on the epoll's own queue and every interest
        pt.add(&ep.wq);
        {
            let ints = ep.interests.lock();
            for i in ints.iter() {
                if let Some(wq) = i.file.ops.poll_wait() {
                    pt.add(wq);
                }
            }
        }
        let ready = ep.ready();
        if !ready.is_empty() {
            pt.cancel();
            pt.clear();
            let n = ready.len().min(maxevents);
            let mut buf = Vec::with_capacity(n * 12);
            for (m, d) in ready.iter().take(n) {
                buf.extend_from_slice(&m.to_le_bytes());
                buf.extend_from_slice(&d.to_le_bytes());
            }
            copy_to_user(events, &buf)?;
            // one-shot interests are disabled after reporting
            let mut ints = ep.interests.lock();
            for i in ints.iter_mut() {
                if i.events & EPOLLONESHOT != 0 && ready.iter().any(|(_, d)| *d == i.data) {
                    i.events &= EPOLLET | EPOLLONESHOT;
                }
            }
            return Ok(n as u64);
        }
        if deadline == u64::MAX {
            pt.cancel();
            return Ok(0);
        }
        match pt.block(deadline) {
            Ok(true) => {
                pt.clear();
                continue;
            }
            Ok(false) => return Ok(0),
            Err(_) => return Err(EINTR),
        }
    }
}

// ---- eventfd -------------------------------------------------------------------

pub struct EventFd {
    count: AtomicU64,
    semaphore: bool,
    wq: WaitQueue,
}

pub fn eventfd(initval: u32, flags: u32) -> SysResult {
    const EFD_SEMAPHORE: u32 = 1;
    const EFD_CLOEXEC: u32 = 0o2000000;
    const EFD_NONBLOCK: u32 = 0o4000;
    let e = Arc::new(EventFd { count: AtomicU64::new(initval as u64), semaphore: flags & EFD_SEMAPHORE != 0, wq: WaitQueue::new() });
    let f = File::new(None, e, crate::fs::vfs::O_RDWR | (flags & EFD_NONBLOCK), "anon_inode:[eventfd]");
    Ok(super::fs::files().alloc(f, flags & EFD_CLOEXEC != 0, 0)? as u64)
}

impl FileOps for EventFd {
    fn read(&self, f: &File, b: &mut [u8]) -> Result<usize> {
        if b.len() < 8 {
            return Err(EINVAL);
        }
        loop {
            let c = self.count.load(Ordering::Relaxed);
            if c > 0 {
                let v = if self.semaphore { 1 } else { c };
                self.count.store(c - v, Ordering::Relaxed);
                b[..8].copy_from_slice(&v.to_le_bytes());
                self.wq.wake_all();
                return Ok(8);
            }
            if f.nonblock() {
                return Err(EAGAIN);
            }
            self.wq.wait_until(|| self.count.load(Ordering::Relaxed) > 0).map_err(|_| EINTR)?;
        }
    }
    fn write(&self, f: &File, b: &[u8]) -> Result<usize> {
        if b.len() < 8 {
            return Err(EINVAL);
        }
        let v = u64::from_le_bytes(b[..8].try_into().unwrap());
        if v == u64::MAX {
            return Err(EINVAL);
        }
        loop {
            let c = self.count.load(Ordering::Relaxed);
            if u64::MAX - 1 - c >= v {
                self.count.store(c + v, Ordering::Relaxed);
                self.wq.wake_all();
                return Ok(8);
            }
            if f.nonblock() {
                return Err(EAGAIN);
            }
            self.wq.wait_until(|| u64::MAX - 1 - self.count.load(Ordering::Relaxed) >= v).map_err(|_| EINTR)?;
        }
    }
    fn poll(&self, _f: &File) -> u32 {
        let c = self.count.load(Ordering::Relaxed);
        let mut m = 0;
        if c > 0 {
            m |= POLLIN;
        }
        if c < u64::MAX - 1 {
            m |= POLLOUT;
        }
        m
    }
    fn poll_wait(&self) -> Option<&WaitQueue> {
        Some(&self.wq)
    }
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
}

// ---- timerfd -------------------------------------------------------------------

pub struct TimerFd {
    clock: i32,
    state: SpinLock<TimerState>,
    wq: WaitQueue,
}

#[derive(Default, Clone, Copy)]
struct TimerState {
    next_ns: u64, // uptime-based, 0 = disarmed
    interval_ns: u64,
    expirations: u64,
}

static TIMERFDS: SpinLock<Vec<alloc::sync::Weak<TimerFd>>> = SpinLock::new(Vec::new());

/// Called from the timer tick: fire expired timerfds.
pub fn timerfd_tick(now: u64) {
    let list = TIMERFDS.lock();
    for w in list.iter() {
        if let Some(t) = w.upgrade() {
            let mut fire = false;
            {
                let mut s = t.state.lock();
                if s.next_ns != 0 && now >= s.next_ns {
                    if s.interval_ns != 0 {
                        let missed = (now - s.next_ns) / s.interval_ns + 1;
                        s.expirations += missed;
                        s.next_ns += missed * s.interval_ns;
                    } else {
                        s.expirations += 1;
                        s.next_ns = 0;
                    }
                    fire = true;
                }
            }
            if fire {
                t.wq.wake_all();
            }
        }
    }
}

pub fn timerfd_create(clock: i32, flags: u32) -> SysResult {
    const TFD_CLOEXEC: u32 = 0o2000000;
    const TFD_NONBLOCK: u32 = 0o4000;
    if clock != 0 && clock != 1 && clock != 7 {
        return Err(EINVAL);
    }
    let t = Arc::new(TimerFd { clock, state: SpinLock::new(TimerState::default()), wq: WaitQueue::new() });
    {
        let mut l = TIMERFDS.lock();
        l.retain(|w| w.strong_count() > 0);
        l.push(Arc::downgrade(&t));
    }
    let f = File::new(None, t, crate::fs::vfs::O_RDWR | (flags & TFD_NONBLOCK), "anon_inode:[timerfd]");
    Ok(super::fs::files().alloc(f, flags & TFD_CLOEXEC != 0, 0)? as u64)
}

fn get_timerfd(fd: i32) -> Result<Arc<TimerFd>> {
    let f = super::fs::files().get(fd)?;
    Arc::downcast::<TimerFd>(f.ops.clone().as_any_arc()).map_err(|_| EINVAL)
}

pub fn timerfd_settime(fd: i32, flags: u32, new: usize, old: usize) -> SysResult {
    let t = get_timerfd(fd)?;
    let interval = super::time::read_timespec(new)?;
    let value = super::time::read_timespec(new + 16)?;
    let now = crate::arch::x86_64::tsc::uptime_ns();
    let mut s = t.state.lock();
    if old != 0 {
        let left = if s.next_ns == 0 { 0 } else { s.next_ns.saturating_sub(now) };
        super::time::write_timespec(old, s.interval_ns)?;
        super::time::write_timespec(old + 16, left)?;
    }
    if value == 0 {
        s.next_ns = 0;
        s.interval_ns = 0;
    } else if flags & 1 != 0 {
        // TFD_TIMER_ABSTIME relative to the clock
        let clock_now = super::time::clock_now(t.clock).unwrap_or(now);
        s.next_ns = now + value.saturating_sub(clock_now);
        s.interval_ns = interval;
    } else {
        s.next_ns = now + value;
        s.interval_ns = interval;
    }
    s.expirations = 0;
    Ok(0)
}

pub fn timerfd_gettime(fd: i32, cur: usize) -> SysResult {
    let t = get_timerfd(fd)?;
    let s = *t.state.lock();
    let now = crate::arch::x86_64::tsc::uptime_ns();
    let left = if s.next_ns == 0 { 0 } else { s.next_ns.saturating_sub(now) };
    super::time::write_timespec(cur, s.interval_ns)?;
    super::time::write_timespec(cur + 16, left)?;
    Ok(0)
}

impl FileOps for TimerFd {
    fn read(&self, f: &File, b: &mut [u8]) -> Result<usize> {
        if b.len() < 8 {
            return Err(EINVAL);
        }
        loop {
            {
                let mut s = self.state.lock();
                if s.expirations > 0 {
                    let v = s.expirations;
                    s.expirations = 0;
                    b[..8].copy_from_slice(&v.to_le_bytes());
                    return Ok(8);
                }
                if s.next_ns == 0 && f.nonblock() {
                    return Err(EAGAIN);
                }
            }
            if f.nonblock() {
                return Err(EAGAIN);
            }
            self.wq.wait_until(|| self.state.lock().expirations > 0).map_err(|_| EINTR)?;
        }
    }
    fn write(&self, _f: &File, _b: &[u8]) -> Result<usize> {
        Err(EINVAL)
    }
    fn poll(&self, _f: &File) -> u32 {
        if self.state.lock().expirations > 0 {
            POLLIN
        } else {
            0
        }
    }
    fn poll_wait(&self) -> Option<&WaitQueue> {
        Some(&self.wq)
    }
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
}
