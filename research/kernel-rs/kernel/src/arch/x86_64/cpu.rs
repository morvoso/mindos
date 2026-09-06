//! CPU identification, control registers and feature enablement.

use super::msr;
use core::arch::asm;
use core::arch::x86_64::{__cpuid_count, CpuidResult};

pub struct CpuFeatures {
    pub vendor: [u8; 12],
    pub brand: [u8; 48],
    pub max_leaf: u32,
    pub max_ext_leaf: u32,
    pub xsave: bool,
    pub avx: bool,
    pub avx2: bool,
    pub avx512f: bool,
    pub fma: bool,
    pub f16c: bool,
    pub fsgsbase: bool,
    pub x2apic: bool,
    pub rdrand: bool,
    pub rdseed: bool,
    pub invariant_tsc: bool,
    pub tsc_deadline: bool,
    pub pge: bool,
    pub nx: bool,
    pub pat: bool,
    pub xsave_size: u32,
    pub xcr0: u64,
    pub family: u32,
    pub model: u32,
}

impl CpuFeatures {
    pub fn vendor(&self) -> &str {
        core::str::from_utf8(&self.vendor).unwrap_or("?")
    }
    pub fn brand(&self) -> &str {
        core::str::from_utf8(&self.brand).unwrap_or("?").trim_end_matches('\0').trim()
    }
}

static FEATURES: crate::sync::Once<CpuFeatures> = crate::sync::Once::new();

#[inline]
pub fn cpuid(leaf: u32, sub: u32) -> CpuidResult {
    __cpuid_count(leaf, sub)
}

pub fn features() -> &'static CpuFeatures {
    FEATURES.get().expect("cpu features not initialised")
}

pub fn init_features() {
    let l0 = cpuid(0, 0);
    let mut vendor = [0u8; 12];
    vendor[0..4].copy_from_slice(&l0.ebx.to_le_bytes());
    vendor[4..8].copy_from_slice(&l0.edx.to_le_bytes());
    vendor[8..12].copy_from_slice(&l0.ecx.to_le_bytes());
    let l1 = cpuid(1, 0);
    let l7 = if l0.eax >= 7 { cpuid(7, 0) } else { CpuidResult { eax: 0, ebx: 0, ecx: 0, edx: 0 } };
    let ext0 = cpuid(0x8000_0000, 0);
    let ext1 = if ext0.eax >= 0x8000_0001 { cpuid(0x8000_0001, 0) } else { CpuidResult { eax: 0, ebx: 0, ecx: 0, edx: 0 } };
    let ext7 = if ext0.eax >= 0x8000_0007 { cpuid(0x8000_0007, 0) } else { CpuidResult { eax: 0, ebx: 0, ecx: 0, edx: 0 } };
    let mut brand = [0u8; 48];
    if ext0.eax >= 0x8000_0004 {
        for (i, leaf) in (0x8000_0002u32..=0x8000_0004).enumerate() {
            let r = cpuid(leaf, 0);
            let o = i * 16;
            brand[o..o + 4].copy_from_slice(&r.eax.to_le_bytes());
            brand[o + 4..o + 8].copy_from_slice(&r.ebx.to_le_bytes());
            brand[o + 8..o + 12].copy_from_slice(&r.ecx.to_le_bytes());
            brand[o + 12..o + 16].copy_from_slice(&r.edx.to_le_bytes());
        }
    }
    let family = ((l1.eax >> 8) & 0xF) + ((l1.eax >> 20) & 0xFF);
    let model = ((l1.eax >> 4) & 0xF) | (((l1.eax >> 16) & 0xF) << 4);
    FEATURES.call_once(|| CpuFeatures {
        vendor,
        brand,
        max_leaf: l0.eax,
        max_ext_leaf: ext0.eax,
        xsave: l1.ecx & (1 << 26) != 0,
        avx: l1.ecx & (1 << 28) != 0,
        avx2: l7.ebx & (1 << 5) != 0,
        avx512f: l7.ebx & (1 << 16) != 0,
        fma: l1.ecx & (1 << 12) != 0,
        f16c: l1.ecx & (1 << 29) != 0,
        fsgsbase: l7.ebx & 1 != 0,
        x2apic: l1.ecx & (1 << 21) != 0,
        rdrand: l1.ecx & (1 << 30) != 0,
        rdseed: l7.ebx & (1 << 18) != 0,
        invariant_tsc: ext7.edx & (1 << 8) != 0,
        tsc_deadline: l1.ecx & (1 << 24) != 0,
        pge: l1.edx & (1 << 13) != 0,
        nx: ext1.edx & (1 << 20) != 0,
        pat: l1.edx & (1 << 16) != 0,
        xsave_size: 512 + 64,
        xcr0: 3,
        family,
        model,
    });
}

#[inline(always)]
pub fn read_cr0() -> u64 {
    let v: u64;
    unsafe { asm!("mov {}, cr0", out(reg) v, options(nomem, nostack, preserves_flags)) };
    v
}
#[inline(always)]
pub fn write_cr0(v: u64) {
    unsafe { asm!("mov cr0, {}", in(reg) v, options(nomem, nostack, preserves_flags)) };
}
#[inline(always)]
pub fn read_cr2() -> u64 {
    let v: u64;
    unsafe { asm!("mov {}, cr2", out(reg) v, options(nomem, nostack, preserves_flags)) };
    v
}
#[inline(always)]
pub fn read_cr3() -> u64 {
    let v: u64;
    unsafe { asm!("mov {}, cr3", out(reg) v, options(nomem, nostack, preserves_flags)) };
    v
}
#[inline(always)]
pub fn write_cr3(v: u64) {
    unsafe { asm!("mov cr3, {}", in(reg) v, options(nostack, preserves_flags)) };
}
#[inline(always)]
pub fn read_cr4() -> u64 {
    let v: u64;
    unsafe { asm!("mov {}, cr4", out(reg) v, options(nomem, nostack, preserves_flags)) };
    v
}
#[inline(always)]
pub fn write_cr4(v: u64) {
    unsafe { asm!("mov cr4, {}", in(reg) v, options(nomem, nostack, preserves_flags)) };
}
#[inline(always)]
pub fn invlpg(va: usize) {
    unsafe { asm!("invlpg [{}]", in(reg) va, options(nostack, preserves_flags)) };
}
#[inline(always)]
pub fn rdtsc() -> u64 {
    let (lo, hi): (u32, u32);
    unsafe { asm!("rdtsc", out("eax") lo, out("edx") hi, options(nomem, nostack, preserves_flags)) };
    ((hi as u64) << 32) | lo as u64
}
#[inline(always)]
pub fn pause() {
    core::hint::spin_loop();
}

pub fn xgetbv(idx: u32) -> u64 {
    let (lo, hi): (u32, u32);
    unsafe { asm!("xgetbv", in("ecx") idx, out("eax") lo, out("edx") hi, options(nomem, nostack, preserves_flags)) };
    ((hi as u64) << 32) | lo as u64
}
pub fn xsetbv(idx: u32, v: u64) {
    unsafe { asm!("xsetbv", in("ecx") idx, in("eax") v as u32, in("edx") (v >> 32) as u32, options(nomem, nostack, preserves_flags)) };
}

/// Enable SSE/AVX for user space and XSAVE-based context switching.
/// The kernel itself is compiled without SIMD, so this only affects user tasks.
pub fn enable_fp_simd() {
    let mut cr0 = read_cr0();
    cr0 &= !(1 << 2); // EM
    cr0 |= 1 << 1; // MP
    cr0 &= !(1 << 3); // TS
    cr0 |= 1 << 5; // NE
    cr0 |= 1 << 16; // WP (kernel honours read-only pages)
    write_cr0(cr0);
    let mut cr4 = read_cr4();
    cr4 |= (1 << 9) | (1 << 10); // OSFXSR, OSXMMEXCPT
    let f = features();
    if f.pge {
        cr4 |= 1 << 7;
    }
    if f.fsgsbase {
        cr4 |= 1 << 16;
    }
    write_cr4(cr4);
    if f.xsave {
        write_cr4(read_cr4() | (1 << 18)); // OSXSAVE
        let supported = {
            let r = cpuid(0xD, 0);
            ((r.edx as u64) << 32) | r.eax as u64
        };
        let mut xcr0 = 0x3u64; // x87 | SSE
        if f.avx {
            xcr0 |= 0x4;
        }
        if f.avx512f {
            xcr0 |= 0xE0; // opmask, ZMM_Hi256, Hi16_ZMM
        }
        xcr0 &= supported;
        xsetbv(0, xcr0);
        let size = cpuid(0xD, 0).ebx; // size for the currently enabled XCR0 set
        // SAFETY: FEATURES is only written here during single-threaded boot.
        let fm = FEATURES.get().unwrap() as *const CpuFeatures as *mut CpuFeatures;
        unsafe {
            (*fm).xsave_size = size.max(576);
            (*fm).xcr0 = xcr0;
        }
    }
    // NX
    if f.nx {
        msr::write(msr::IA32_EFER, msr::read(msr::IA32_EFER) | (1 << 11));
    }
}

/// Program the PAT so that PWT=1 selects write-combining (used for framebuffers).
/// Entries: 0 WB, 1 WC, 2 UC-, 3 UC, 4 WB, 5 WT, 6 UC-, 7 UC.
pub fn init_pat() {
    if features().pat {
        msr::write(msr::IA32_PAT, 0x0007_0406_0007_0106);
    }
}

/// Save extended state into a 64-byte aligned XSAVE area.
#[inline]
pub unsafe fn xsave(area: *mut u8) {
    let f = features();
    if f.xsave {
        unsafe { asm!("xsave [{}]", in(reg) area, in("eax") f.xcr0 as u32, in("edx") (f.xcr0 >> 32) as u32, options(nostack, preserves_flags)) };
    } else {
        unsafe { asm!("fxsave [{}]", in(reg) area, options(nostack, preserves_flags)) };
    }
}
#[inline]
pub unsafe fn xrstor(area: *const u8) {
    let f = features();
    if f.xsave {
        unsafe { asm!("xrstor [{}]", in(reg) area, in("eax") f.xcr0 as u32, in("edx") (f.xcr0 >> 32) as u32, options(nostack, preserves_flags)) };
    } else {
        unsafe { asm!("fxrstor [{}]", in(reg) area, options(nostack, preserves_flags)) };
    }
}

/// Initialise an XSAVE area to the architectural init state (MXCSR 0x1F80, x87 CW 0x37F).
pub fn init_fpu_area(area: &mut [u8]) {
    for b in area.iter_mut() {
        *b = 0;
    }
    // FXSAVE legacy region: FCW at 0, MXCSR at 24
    area[0] = 0x7F;
    area[1] = 0x03;
    area[24..28].copy_from_slice(&0x1F80u32.to_le_bytes());
    // XSAVE header (offset 512): XSTATE_BV = 0 means every component is in init state
}

pub fn rdrand64() -> Option<u64> {
    if !features().rdrand {
        return None;
    }
    for _ in 0..16 {
        let v: u64;
        let ok: u8;
        unsafe { asm!("rdrand {}; setc {}", out(reg) v, out(reg_byte) ok, options(nomem, nostack)) };
        if ok != 0 {
            return Some(v);
        }
    }
    None
}
