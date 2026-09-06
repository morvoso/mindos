//! uname, resource limits, prctl and friends.

use super::SysResult;
use crate::mm::errno::*;
use crate::mm::user::{copy_from_user, copy_to_user, read_cstr, read_user, write_user};
use crate::proc::process::{RLIMIT_COUNT, RLIM_INFINITY};
use crate::sync::SpinLock;
use alloc::string::String;
use core::sync::atomic::Ordering;

pub const RELEASE: &str = "6.6.0-mindos";
static HOSTNAME: SpinLock<Option<String>> = SpinLock::new(None);

pub fn hostname() -> String {
    HOSTNAME.lock().clone().unwrap_or_else(|| String::from("mindos"))
}
pub fn set_hostname(s: &str) {
    *HOSTNAME.lock() = Some(String::from(s));
}

pub fn uname(buf: usize) -> SysResult {
    let mut out = [0u8; 65 * 6];
    let put = |out: &mut [u8], i: usize, s: &str| {
        let b = s.as_bytes();
        let n = b.len().min(64);
        out[i * 65..i * 65 + n].copy_from_slice(&b[..n]);
    };
    put(&mut out, 0, "Linux");
    put(&mut out, 1, &hostname());
    put(&mut out, 2, RELEASE);
    put(&mut out, 3, &alloc::format!("#1 SMP MindOS {}", crate::VERSION));
    put(&mut out, 4, "x86_64");
    put(&mut out, 5, "(none)");
    copy_to_user(buf, &out)?;
    Ok(0)
}

pub fn sethostname(name: usize, len: usize) -> SysResult {
    if len > 64 {
        return Err(EINVAL);
    }
    let mut b = alloc::vec![0u8; len];
    copy_from_user(&mut b, name)?;
    set_hostname(core::str::from_utf8(&b).map_err(|_| EINVAL)?);
    Ok(0)
}

pub fn getrlimit(which: usize, buf: usize) -> SysResult {
    if which >= RLIMIT_COUNT {
        return Err(EINVAL);
    }
    let (s, h) = crate::sched::current_ref().proc.rlimit(which);
    write_user::<[u64; 2]>(buf, [s, h])?;
    Ok(0)
}

pub fn setrlimit(which: usize, buf: usize) -> SysResult {
    if which >= RLIMIT_COUNT {
        return Err(EINVAL);
    }
    let [s, h]: [u64; 2] = read_user(buf)?;
    if s > h {
        return Err(EINVAL);
    }
    crate::sched::current_ref().proc.rlimits.lock()[which] = (s, h);
    Ok(0)
}

pub fn prlimit64(pid: u32, which: usize, new: usize, old: usize) -> SysResult {
    if which >= RLIMIT_COUNT {
        return Err(EINVAL);
    }
    let p = if pid == 0 { crate::sched::current_ref().proc.clone() } else { crate::proc::process::lookup(pid).ok_or(ESRCH)? };
    if old != 0 {
        let (s, h) = p.rlimit(which);
        write_user::<[u64; 2]>(old, [s, h])?;
    }
    if new != 0 {
        let [s, h]: [u64; 2] = read_user(new)?;
        if s > h {
            return Err(EINVAL);
        }
        p.rlimits.lock()[which] = (s, h);
    }
    Ok(0)
}

pub fn getrusage(who: i32, buf: usize) -> SysResult {
    let p = &crate::sched::current_ref().proc;
    let cpu: u64 = match who {
        0 => p.threads.lock().iter().map(|t| t.cpu_time_ns.load(Ordering::Relaxed)).sum(),
        1 => crate::sched::current_ref().cpu_time_ns.load(Ordering::Relaxed),
        -1 => 0,
        _ => return Err(EINVAL),
    };
    let mut out = [0i64; 18];
    out[0] = (cpu / 1_000_000_000) as i64;
    out[1] = ((cpu % 1_000_000_000) / 1000) as i64;
    out[4] = p.aspace().map(|a| a.rss() as i64 * 4).unwrap_or(0); // ru_maxrss in KiB
    let bytes = unsafe { core::slice::from_raw_parts(out.as_ptr() as *const u8, 144) };
    copy_to_user(buf, bytes)?;
    Ok(0)
}

pub fn sysinfo(buf: usize) -> SysResult {
    let (total, free) = crate::mm::pmm::stats();
    let mut out = [0u8; 112];
    let up = crate::arch::x86_64::tsc::uptime_ns() / 1_000_000_000;
    out[0..8].copy_from_slice(&(up as i64).to_le_bytes());
    // loads[3] at 8..32 (zero)
    out[32..40].copy_from_slice(&((total * 4096) as u64).to_le_bytes());
    out[40..48].copy_from_slice(&((free * 4096) as u64).to_le_bytes());
    // sharedram, bufferram, totalswap, freeswap = 0
    out[80..82].copy_from_slice(&(crate::proc::process::count() as u16).to_le_bytes());
    // totalhigh, freehigh = 0; mem_unit
    out[104..108].copy_from_slice(&1u32.to_le_bytes());
    copy_to_user(buf, &out)?;
    Ok(0)
}

pub fn syslog(typ: i32, buf: usize, len: usize) -> SysResult {
    match typ {
        2 | 3 | 4 => {
            let mut v = alloc::vec![0u8; len.min(128 * 1024)];
            let (n, _) = crate::console::CONSOLE.lock().log.read_from(0, &mut v);
            copy_to_user(buf, &v[..n])?;
            Ok(n as u64)
        }
        9 | 10 => Ok(crate::console::CONSOLE.lock().log.len() as u64),
        _ => Ok(0),
    }
}

pub fn prctl(option: i32, a2: u64, _a3: u64, _a4: u64, _a5: u64) -> SysResult {
    const PR_SET_PDEATHSIG: i32 = 1;
    const PR_GET_PDEATHSIG: i32 = 2;
    const PR_GET_DUMPABLE: i32 = 3;
    const PR_SET_DUMPABLE: i32 = 4;
    const PR_SET_NAME: i32 = 15;
    const PR_GET_NAME: i32 = 16;
    const PR_SET_SECCOMP: i32 = 22;
    const PR_SET_NO_NEW_PRIVS: i32 = 38;
    const PR_GET_NO_NEW_PRIVS: i32 = 39;
    const PR_SET_VMA: i32 = 0x53564d41;
    let t = crate::sched::current_ref();
    match option {
        PR_SET_NAME => {
            let s = read_cstr(a2 as usize, 16).unwrap_or_default();
            let s: String = s.chars().take(15).collect();
            *t.name.lock() = s.clone();
            *t.proc.comm.lock() = s;
            Ok(0)
        }
        PR_GET_NAME => {
            let mut b = [0u8; 16];
            let n = t.proc.comm();
            let bytes = n.as_bytes();
            let l = bytes.len().min(15);
            b[..l].copy_from_slice(&bytes[..l]);
            copy_to_user(a2 as usize, &b)?;
            Ok(0)
        }
        PR_SET_PDEATHSIG | PR_SET_DUMPABLE | PR_SET_NO_NEW_PRIVS | PR_SET_VMA | PR_SET_SECCOMP => Ok(0),
        PR_GET_PDEATHSIG => {
            write_user::<i32>(a2 as usize, 0)?;
            Ok(0)
        }
        PR_GET_DUMPABLE => Ok(1),
        PR_GET_NO_NEW_PRIVS => Ok(0),
        _ => Err(EINVAL),
    }
}

pub fn arch_prctl(code: i32, addr: u64) -> SysResult {
    const ARCH_SET_GS: i32 = 0x1001;
    const ARCH_SET_FS: i32 = 0x1002;
    const ARCH_GET_FS: i32 = 0x1003;
    const ARCH_GET_GS: i32 = 0x1004;
    const ARCH_GET_CPUID: i32 = 0x1011;
    const ARCH_SET_CPUID: i32 = 0x1012;
    let t = crate::sched::current_ref();
    match code {
        ARCH_SET_FS => {
            if addr >= crate::mm::USER_TOP as u64 {
                return Err(EPERM);
            }
            t.fs_base.store(addr, Ordering::Relaxed);
            crate::arch::x86_64::msr::write(crate::arch::x86_64::msr::IA32_FS_BASE, addr);
            Ok(0)
        }
        ARCH_GET_FS => {
            write_user::<u64>(addr as usize, t.fs_base.load(Ordering::Relaxed))?;
            Ok(0)
        }
        ARCH_SET_GS => Err(EINVAL),
        ARCH_GET_GS => {
            write_user::<u64>(addr as usize, 0)?;
            Ok(0)
        }
        ARCH_GET_CPUID => Ok(1),
        ARCH_SET_CPUID => Ok(0),
        _ => Err(EINVAL),
    }
}

pub fn getrandom(buf: usize, len: usize, _flags: u32) -> SysResult {
    let len = len.min(1 << 20);
    let mut v = alloc::vec![0u8; len];
    crate::dev::chardev::fill_random(&mut v);
    copy_to_user(buf, &v)?;
    Ok(len as u64)
}

pub fn sched_getaffinity(len: usize, mask: usize) -> SysResult {
    if len < 8 {
        return Err(EINVAL);
    }
    let n = crate::arch::x86_64::percpu::count().max(1);
    let bits: u64 = if n >= 64 { u64::MAX } else { (1u64 << n) - 1 };
    let mut out = alloc::vec![0u8; len.min(128)];
    out[..8].copy_from_slice(&bits.to_le_bytes());
    copy_to_user(mask, &out)?;
    Ok(out.len() as u64)
}

pub fn reboot(cmd: u32) -> SysResult {
    match cmd {
        0x01234567 | 0xA1B2C3D4 => {
            klog!("sys", "reboot requested");
            crate::arch::x86_64::reboot()
        }
        0x4321FEDC => {
            klog!("sys", "power off requested");
            crate::arch::x86_64::power_off()
        }
        0xCDEF0123 => {
            klog!("sys", "halt requested");
            crate::arch::x86_64::power_off()
        }
        0x89ABCDEF | 0 => Ok(0), // CAD on/off
        _ => Err(EINVAL),
    }
}

pub fn _unused() {
    let _ = RLIM_INFINITY;
}
