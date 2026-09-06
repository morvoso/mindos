//! ACPI table discovery: RSDP -> XSDT/RSDT -> MADT (and a few others).

use crate::boot::limine::BootInfo;
use crate::mm::vmm::p2v;
use crate::sync::Once;
use alloc::vec::Vec;

#[repr(C, packed)]
struct Rsdp {
    signature: [u8; 8],
    checksum: u8,
    oemid: [u8; 6],
    revision: u8,
    rsdt_addr: u32,
    length: u32,
    xsdt_addr: u64,
    ext_checksum: u8,
    reserved: [u8; 3],
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct SdtHeader {
    pub signature: [u8; 4],
    pub length: u32,
    pub revision: u8,
    pub checksum: u8,
    pub oemid: [u8; 6],
    pub oem_table_id: [u8; 8],
    pub oem_revision: u32,
    pub creator_id: u32,
    pub creator_revision: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct IoApicEntry {
    pub id: u8,
    pub address: u32,
    pub gsi_base: u32,
}
#[derive(Clone, Copy, Debug)]
pub struct IsaOverride {
    pub source: u8,
    pub gsi: u32,
    pub flags: u16,
}
#[derive(Clone, Copy, Debug)]
pub struct CpuEntry {
    pub acpi_id: u32,
    pub lapic_id: u32,
    pub enabled: bool,
}

pub struct AcpiInfo {
    pub lapic_addr: u64,
    pub cpus: Vec<CpuEntry>,
    pub ioapics: Vec<IoApicEntry>,
    pub overrides: Vec<IsaOverride>,
    pub tables: Vec<(&'static SdtHeader, usize)>,
    pub has_8042: bool,
    pub pm1a_cnt: u32,
    pub century_reg: u8,
}

static INFO: Once<AcpiInfo> = Once::new();

pub fn info() -> &'static AcpiInfo {
    INFO.get().expect("acpi not initialised")
}

fn checksum_ok(p: *const u8, len: usize) -> bool {
    let mut s: u8 = 0;
    for i in 0..len {
        s = s.wrapping_add(unsafe { *p.add(i) });
    }
    s == 0
}

fn header_at(phys: u64) -> Option<&'static SdtHeader> {
    if phys == 0 {
        return None;
    }
    let h = unsafe { &*(p2v(phys) as *const SdtHeader) };
    let len = h.length as usize;
    if len < core::mem::size_of::<SdtHeader>() || len > 1 << 24 {
        return None;
    }
    Some(h)
}

pub fn init(boot: &BootInfo) {
    let mut info = AcpiInfo { lapic_addr: 0xFEE0_0000, cpus: Vec::new(), ioapics: Vec::new(), overrides: Vec::new(), tables: Vec::new(), has_8042: true, pm1a_cnt: 0, century_reg: 0 };
    if boot.rsdp_phys == 0 {
        klog!("acpi", "no RSDP from bootloader; assuming legacy defaults");
        INFO.call_once(|| info);
        return;
    }
    let rsdp = unsafe { &*(p2v(boot.rsdp_phys) as *const Rsdp) };
    let sig = rsdp.signature;
    if &sig != b"RSD PTR " {
        klog!("acpi", "bad RSDP signature");
        INFO.call_once(|| info);
        return;
    }
    let rev = rsdp.revision;
    let xsdt = if rev >= 2 { rsdp.xsdt_addr } else { 0 };
    let rsdt = rsdp.rsdt_addr as u64;

    let mut tables: Vec<(&'static SdtHeader, usize)> = Vec::new();
    if let Some(h) = header_at(xsdt) {
        let n = (h.length as usize - 36) / 8;
        let base = (h as *const SdtHeader as usize) + 36;
        for i in 0..n {
            let pa = unsafe { core::ptr::read_unaligned((base + i * 8) as *const u64) };
            if let Some(t) = header_at(pa) {
                tables.push((t, pa as usize));
            }
        }
    } else if let Some(h) = header_at(rsdt) {
        let n = (h.length as usize - 36) / 4;
        let base = (h as *const SdtHeader as usize) + 36;
        for i in 0..n {
            let pa = unsafe { core::ptr::read_unaligned((base + i * 4) as *const u32) } as u64;
            if let Some(t) = header_at(pa) {
                tables.push((t, pa as usize));
            }
        }
    }

    for (t, _pa) in tables.iter() {
        let sig = t.signature;
        let len = t.length as usize;
        let base = *t as *const SdtHeader as usize;
        let ok = checksum_ok(base as *const u8, len);
        klog!("acpi", "table {} rev {} len {} {}", core::str::from_utf8(&sig).unwrap_or("?"), t.revision, len, if ok { "" } else { "(bad checksum)" });
        match &sig {
            b"APIC" => parse_madt(base, len, &mut info),
            b"FACP" => {
                // boot architecture flags at offset 109 (ACPI 2+), pm1a_cnt_blk at 64, century at 108
                if len >= 111 {
                    let flags = unsafe { core::ptr::read_unaligned((base + 109) as *const u16) };
                    info.has_8042 = t.revision < 2 || flags & 2 != 0;
                    info.century_reg = unsafe { *((base + 108) as *const u8) };
                }
                if len >= 68 {
                    info.pm1a_cnt = unsafe { core::ptr::read_unaligned((base + 64) as *const u32) };
                }
            }
            _ => {}
        }
    }
    info.tables = tables;
    klog!("acpi", "{} cpus, {} ioapics, {} irq overrides, lapic at {:#x}", info.cpus.len(), info.ioapics.len(), info.overrides.len(), info.lapic_addr);
    INFO.call_once(|| info);
}

fn parse_madt(base: usize, len: usize, info: &mut AcpiInfo) {
    let rd32 = |o: usize| unsafe { core::ptr::read_unaligned((base + o) as *const u32) };
    let rd16 = |o: usize| unsafe { core::ptr::read_unaligned((base + o) as *const u16) };
    let rd8 = |o: usize| unsafe { *((base + o) as *const u8) };
    info.lapic_addr = rd32(36) as u64;
    let mut p = 44;
    while p + 2 <= len {
        let typ = rd8(p);
        let l = rd8(p + 1) as usize;
        if l < 2 || p + l > len {
            break;
        }
        match typ {
            0 => {
                let flags = rd32(p + 4);
                info.cpus.push(CpuEntry { acpi_id: rd8(p + 2) as u32, lapic_id: rd8(p + 3) as u32, enabled: flags & 1 != 0 || flags & 2 != 0 });
            }
            1 => info.ioapics.push(IoApicEntry { id: rd8(p + 2), address: rd32(p + 4), gsi_base: rd32(p + 8) }),
            2 => info.overrides.push(IsaOverride { source: rd8(p + 3), gsi: rd32(p + 4), flags: rd16(p + 8) }),
            5 => {
                info.lapic_addr = unsafe { core::ptr::read_unaligned((base + p + 4) as *const u64) };
            }
            9 => {
                let flags = rd32(p + 8);
                info.cpus.push(CpuEntry { acpi_id: rd32(p + 12), lapic_id: rd32(p + 4), enabled: flags & 1 != 0 });
            }
            _ => {}
        }
        p += l;
    }
}

/// Find a table by signature (address of its header in the direct map).
pub fn find_table(sig: &[u8; 4]) -> Option<&'static SdtHeader> {
    info().tables.iter().find(|(t, _)| &t.signature == sig).map(|(t, _)| *t)
}
