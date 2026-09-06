//! Limine boot protocol bindings (base revision 5) and the boot-information
//! snapshot the kernel keeps for the rest of its life.
//!
//! All response memory lives in bootloader-reclaimable regions, which MindOS
//! never reclaims, so the pointers stay valid.

use core::cell::UnsafeCell;
use core::ffi::{c_char, CStr};
use core::ptr;

const COMMON_MAGIC: [u64; 2] = [0xc7b1dd30df4c8b88, 0x0a82e883a194f07b];

/// Base revision tag. The bootloader zeroes the third word when it supports
/// the requested revision and (rev >= 3) writes the loaded revision to the second.
#[repr(C)]
pub struct BaseRevision(UnsafeCell<[u64; 3]>);
unsafe impl Sync for BaseRevision {}
impl BaseRevision {
    pub const fn new(rev: u64) -> Self {
        BaseRevision(UnsafeCell::new([0xf9562b2d5c95a6c8, 0x6a7b384944536bdc, rev]))
    }
    fn words(&self) -> [u64; 3] {
        unsafe { ptr::read_volatile(self.0.get()) }
    }
    pub fn supported(&self) -> bool {
        self.words()[2] == 0
    }
    /// Revision the bootloader actually used, if it reported one.
    pub fn loaded_revision(&self) -> Option<u64> {
        let w = self.words();
        if w[1] != 0x6a7b384944536bdc {
            Some(w[1])
        } else {
            None
        }
    }
}

#[repr(C)]
pub struct Request<R, X = ()> {
    id: [u64; 4],
    revision: u64,
    response: UnsafeCell<*const R>,
    extra: X,
}
unsafe impl<R, X> Sync for Request<R, X> {}
impl<R, X> Request<R, X> {
    pub const fn with(id: [u64; 2], revision: u64, extra: X) -> Self {
        Request {
            id: [COMMON_MAGIC[0], COMMON_MAGIC[1], id[0], id[1]],
            revision,
            response: UnsafeCell::new(ptr::null()),
            extra,
        }
    }
    pub fn response(&self) -> Option<&'static R> {
        let p = unsafe { ptr::read_volatile(self.response.get()) };
        if p.is_null() {
            None
        } else {
            Some(unsafe { &*p })
        }
    }
}
impl<R> Request<R, ()> {
    pub const fn new(id: [u64; 2], revision: u64) -> Self {
        Self::with(id, revision, ())
    }
}

// ---- response structures ---------------------------------------------------

#[repr(C)]
pub struct BootloaderInfoResponse {
    pub revision: u64,
    pub name: *const c_char,
    pub version: *const c_char,
}

#[repr(C)]
pub struct ExecutableCmdlineResponse {
    pub revision: u64,
    pub cmdline: *const c_char,
}

#[repr(C)]
pub struct StackSizeResponse {
    pub revision: u64,
}

#[repr(C)]
pub struct HhdmResponse {
    pub revision: u64,
    pub offset: u64,
}

#[repr(C)]
pub struct VideoMode {
    pub pitch: u64,
    pub width: u64,
    pub height: u64,
    pub bpp: u16,
    pub memory_model: u8,
    pub red_mask_size: u8,
    pub red_mask_shift: u8,
    pub green_mask_size: u8,
    pub green_mask_shift: u8,
    pub blue_mask_size: u8,
    pub blue_mask_shift: u8,
}

#[repr(C)]
pub struct Framebuffer {
    pub address: *mut u8,
    pub width: u64,
    pub height: u64,
    pub pitch: u64,
    pub bpp: u16,
    pub memory_model: u8,
    pub red_mask_size: u8,
    pub red_mask_shift: u8,
    pub green_mask_size: u8,
    pub green_mask_shift: u8,
    pub blue_mask_size: u8,
    pub blue_mask_shift: u8,
    pub unused: [u8; 7],
    pub edid_size: u64,
    pub edid: *const u8,
    // response revision 1
    pub mode_count: u64,
    pub modes: *const *const VideoMode,
}

#[repr(C)]
pub struct FramebufferResponse {
    pub revision: u64,
    pub framebuffer_count: u64,
    pub framebuffers: *const *const Framebuffer,
}

pub const MEMMAP_USABLE: u64 = 0;
pub const MEMMAP_RESERVED: u64 = 1;
pub const MEMMAP_ACPI_RECLAIMABLE: u64 = 2;
pub const MEMMAP_ACPI_NVS: u64 = 3;
pub const MEMMAP_BAD_MEMORY: u64 = 4;
pub const MEMMAP_BOOTLOADER_RECLAIMABLE: u64 = 5;
pub const MEMMAP_EXECUTABLE_AND_MODULES: u64 = 6;
pub const MEMMAP_FRAMEBUFFER: u64 = 7;
pub const MEMMAP_RESERVED_MAPPED: u64 = 8;

#[repr(C)]
pub struct MemmapEntry {
    pub base: u64,
    pub length: u64,
    pub typ: u64,
}

#[repr(C)]
pub struct MemmapResponse {
    pub revision: u64,
    pub entry_count: u64,
    pub entries: *const *const MemmapEntry,
}

#[repr(C)]
pub struct ExecutableAddressResponse {
    pub revision: u64,
    pub physical_base: u64,
    pub virtual_base: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Uuid {
    pub a: u32,
    pub b: u16,
    pub c: u16,
    pub d: [u8; 8],
}

#[repr(C)]
pub struct File {
    pub revision: u64,
    pub address: *mut u8,
    pub size: u64,
    pub path: *const c_char,
    pub string: *const c_char,
    pub media_type: u32,
    pub unused: u32,
    pub tftp_ipv4: [u8; 4],
    pub tftp_port: u32,
    pub partition_index: u32,
    pub mbr_disk_id: u32,
    pub gpt_disk_uuid: Uuid,
    pub gpt_part_uuid: Uuid,
    pub part_uuid: Uuid,
}

#[repr(C)]
pub struct ModuleResponse {
    pub revision: u64,
    pub module_count: u64,
    pub modules: *const *const File,
}

#[repr(C)]
pub struct RsdpResponse {
    pub revision: u64,
    pub address: u64,
}

#[repr(C)]
pub struct MpInfo {
    pub processor_id: u32,
    pub lapic_id: u32,
    pub reserved: u64,
    pub goto_address: core::sync::atomic::AtomicU64,
    pub extra_argument: core::sync::atomic::AtomicU64,
}

#[repr(C)]
pub struct MpResponse {
    pub revision: u64,
    pub flags: u32,
    pub bsp_lapic_id: u32,
    pub cpu_count: u64,
    pub cpus: *const *const MpInfo,
}

#[repr(C)]
pub struct DateAtBootResponse {
    pub revision: u64,
    pub timestamp: i64,
}

// ---- the requests this kernel makes -----------------------------------------

#[used]
#[link_section = ".limine_requests_start"]
static START_MARKER: [u64; 4] = [0xf6b8f4b39de7d1ae, 0xfab91a6940fcb9cf, 0x785c6ed015d3e316, 0x181e920a7852b9d9];

#[used]
#[link_section = ".limine_requests"]
static BASE_REVISION: BaseRevision = BaseRevision::new(5);

#[used]
#[link_section = ".limine_requests"]
static BOOTLOADER_INFO: Request<BootloaderInfoResponse> = Request::new([0xf55038d8e2a1202f, 0x279426fcf5f59740], 0);

#[used]
#[link_section = ".limine_requests"]
static CMDLINE: Request<ExecutableCmdlineResponse> = Request::new([0x4b161536e598651e, 0xb390ad4a2f1f303a], 0);

#[used]
#[link_section = ".limine_requests"]
static STACK_SIZE: Request<StackSizeResponse, u64> = Request::with([0x224ef0460a8e8926, 0xe1cb0fc25f46ea3d], 0, 256 * 1024);

#[used]
#[link_section = ".limine_requests"]
static HHDM: Request<HhdmResponse> = Request::new([0x48dcf1cb8ad2b852, 0x63984e959a98244b], 0);

#[used]
#[link_section = ".limine_requests"]
static FRAMEBUFFER: Request<FramebufferResponse> = Request::new([0x9d5827dcd881dd75, 0xa3148604f6fab11b], 1);

#[used]
#[link_section = ".limine_requests"]
static MEMMAP: Request<MemmapResponse> = Request::new([0x67cf3d9d378a806f, 0xe304acdfc50c3c62], 0);

#[used]
#[link_section = ".limine_requests"]
static EXEC_ADDR: Request<ExecutableAddressResponse> = Request::new([0x71ba76863cc55f63, 0xb2644a48c516a487], 0);

#[used]
#[link_section = ".limine_requests"]
static MODULES: Request<ModuleResponse> = Request::new([0x3e7e279702be32af, 0xca1c4f3bd1280cee], 0);

#[used]
#[link_section = ".limine_requests"]
static RSDP: Request<RsdpResponse> = Request::new([0xc5e77b6b397e7b43, 0x27637845accdcf3c], 0);

#[used]
#[link_section = ".limine_requests"]
static MP: Request<MpResponse, u64> = Request::with([0x95a67b819a1b857e, 0xa0b61b723b6a73e0], 0, 0);

#[used]
#[link_section = ".limine_requests"]
static DATE_AT_BOOT: Request<DateAtBootResponse> = Request::new([0x502746e184c088aa, 0xfbc5ec83e6327893], 0);

#[used]
#[link_section = ".limine_requests_end"]
static END_MARKER: [u64; 2] = [0xadc0e0531bb10d03, 0x9572709f31764c62];

// ---- boot info snapshot -------------------------------------------------------

#[derive(Clone, Copy, Debug)]
pub struct FbInfo {
    pub phys: u64,
    pub virt: usize,
    pub width: u32,
    pub height: u32,
    pub pitch: u32,
    pub bpp: u32,
    pub red_shift: u8,
    pub green_shift: u8,
    pub blue_shift: u8,
}

#[derive(Clone, Copy)]
pub struct Module {
    pub name: &'static str,
    pub path: &'static str,
    pub virt: usize,
    pub phys: u64,
    pub size: usize,
}

pub struct BootInfo {
    pub bootloader: &'static str,
    pub bootloader_version: &'static str,
    pub base_revision_ok: bool,
    pub loaded_revision: Option<u64>,
    pub hhdm: usize,
    pub memmap: &'static [*const MemmapEntry],
    pub kernel_phys: u64,
    pub kernel_virt: u64,
    pub fb: Option<FbInfo>,
    pub modules: [Option<Module>; 8],
    pub module_count: usize,
    pub rsdp_phys: u64,
    pub cmdline: &'static str,
    pub boot_unix_time: i64,
    pub mp: Option<&'static MpResponse>,
}
unsafe impl Sync for BootInfo {}
unsafe impl Send for BootInfo {}

static BOOT_INFO: crate::sync::Once<BootInfo> = crate::sync::Once::new();

fn cstr(p: *const c_char) -> &'static str {
    if p.is_null() {
        return "";
    }
    unsafe { CStr::from_ptr(p).to_str().unwrap_or("") }
}

/// Collect everything we need from the bootloader. Called exactly once, first
/// thing at boot; panics if a mandatory response is missing.
pub fn init() -> &'static BootInfo {
    let hhdm = HHDM.response().expect("limine: no HHDM response").offset as usize;
    let mm = MEMMAP.response().expect("limine: no memory map");
    let memmap: &'static [*const MemmapEntry] = unsafe { core::slice::from_raw_parts(mm.entries, mm.entry_count as usize) };
    let ea = EXEC_ADDR.response().expect("limine: no executable address");

    let fb = FRAMEBUFFER.response().and_then(|r| {
        if r.framebuffer_count == 0 {
            return None;
        }
        let f = unsafe { &**r.framebuffers };
        let virt = f.address as usize;
        Some(FbInfo {
            phys: (virt - hhdm) as u64,
            virt,
            width: f.width as u32,
            height: f.height as u32,
            pitch: f.pitch as u32,
            bpp: f.bpp as u32,
            red_shift: f.red_mask_shift,
            green_shift: f.green_mask_shift,
            blue_shift: f.blue_mask_shift,
        })
    });

    let mut modules: [Option<Module>; 8] = [None; 8];
    let mut module_count = 0;
    if let Some(r) = MODULES.response() {
        for i in 0..(r.module_count as usize).min(8) {
            let f = unsafe { &**r.modules.add(i) };
            let virt = f.address as usize;
            modules[i] = Some(Module {
                name: cstr(f.string),
                path: cstr(f.path),
                virt,
                phys: (virt - hhdm) as u64,
                size: f.size as usize,
            });
            module_count += 1;
        }
    }

    let loaded_revision = BASE_REVISION.loaded_revision();
    let rsdp_phys = RSDP
        .response()
        .map(|r| {
            let a = r.address;
            // Base revision 3 hands us a physical address; 4+ a virtual (HHDM) one.
            if (a as usize) >= hhdm {
                a - hhdm as u64
            } else {
                a
            }
        })
        .unwrap_or(0);

    let (bl_name, bl_ver) = BOOTLOADER_INFO.response().map(|r| (cstr(r.name), cstr(r.version))).unwrap_or(("unknown", "?"));

    BOOT_INFO.call_once(|| BootInfo {
        bootloader: bl_name,
        bootloader_version: bl_ver,
        base_revision_ok: BASE_REVISION.supported(),
        loaded_revision,
        hhdm,
        memmap,
        kernel_phys: ea.physical_base,
        kernel_virt: ea.virtual_base,
        fb,
        modules,
        module_count,
        rsdp_phys,
        cmdline: CMDLINE.response().map(|r| cstr(r.cmdline)).unwrap_or(""),
        boot_unix_time: DATE_AT_BOOT.response().map(|r| r.timestamp).unwrap_or(0),
        mp: MP.response(),
    })
}

pub fn info() -> &'static BootInfo {
    BOOT_INFO.get().expect("boot info not initialised")
}

impl BootInfo {
    pub fn module(&self, name: &str) -> Option<&Module> {
        self.modules[..self.module_count].iter().flatten().find(|m| m.name == name)
    }
    pub fn memmap_iter(&self) -> impl Iterator<Item = &'static MemmapEntry> + '_ {
        self.memmap.iter().map(|p| unsafe { &**p })
    }
}
