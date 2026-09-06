//! Wall-clock time: boot time from the bootloader plus the TSC uptime.

use core::sync::atomic::{AtomicI64, Ordering};

static BOOT_UNIX_NS: AtomicI64 = AtomicI64::new(0);

pub fn init(boot_unix_time: i64) {
    let t = if boot_unix_time > 0 { boot_unix_time } else { read_cmos_unix() };
    BOOT_UNIX_NS.store(t * 1_000_000_000, Ordering::Relaxed);
}

pub fn wall_time_ns() -> u64 {
    let base = BOOT_UNIX_NS.load(Ordering::Relaxed);
    (base as u64).wrapping_add(crate::arch::x86_64::tsc::uptime_ns())
}

pub fn set_wall_time_ns(ns: u64) {
    let up = crate::arch::x86_64::tsc::uptime_ns();
    BOOT_UNIX_NS.store(ns.wrapping_sub(up) as i64, Ordering::Relaxed);
}

fn cmos(reg: u8) -> u8 {
    use crate::arch::x86_64::io::{inb, outb};
    unsafe {
        outb(0x70, reg);
        inb(0x71)
    }
}

fn bcd(v: u8) -> u8 {
    (v & 0x0f) + (v >> 4) * 10
}

/// Read the CMOS RTC and convert to a unix timestamp (fallback only).
pub fn read_cmos_unix() -> i64 {
    // wait until no update in progress
    for _ in 0..100000 {
        if cmos(0x0A) & 0x80 == 0 {
            break;
        }
    }
    let regb = cmos(0x0B);
    let mut sec = cmos(0x00);
    let mut min = cmos(0x02);
    let mut hour = cmos(0x04);
    let mut day = cmos(0x07);
    let mut mon = cmos(0x08);
    let mut year = cmos(0x09);
    let century = cmos(0x32);
    if regb & 0x04 == 0 {
        sec = bcd(sec);
        min = bcd(min);
        hour = bcd(hour & 0x7f) | (hour & 0x80);
        day = bcd(day);
        mon = bcd(mon);
        year = bcd(year);
    }
    if regb & 0x02 == 0 && hour & 0x80 != 0 {
        hour = ((hour & 0x7f) + 12) % 24;
    }
    let mut full_year = year as i64 + 2000;
    if century != 0 && century != 0xff {
        let c = if regb & 0x04 == 0 { bcd(century) } else { century };
        full_year = c as i64 * 100 + year as i64;
    }
    days_from_civil(full_year, mon as i64, day as i64) * 86400 + hour as i64 * 3600 + min as i64 * 60 + sec as i64
}

/// Days since 1970-01-01 for a proleptic Gregorian date.
pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}
