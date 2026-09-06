//! /dev/fb0: the boot framebuffer as a Linux fbdev-compatible device.

use crate::fs::vfs::{File, FileOps};
use crate::mm::addrspace::Backing;
use crate::mm::errno::*;
use crate::mm::user::copy_to_user;
use alloc::sync::Arc;

pub struct FbDev;

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct FbBitfield {
    offset: u32,
    length: u32,
    msb_right: u32,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct FbVarScreeninfo {
    xres: u32,
    yres: u32,
    xres_virtual: u32,
    yres_virtual: u32,
    xoffset: u32,
    yoffset: u32,
    bits_per_pixel: u32,
    grayscale: u32,
    red: FbBitfield,
    green: FbBitfield,
    blue: FbBitfield,
    transp: FbBitfield,
    nonstd: u32,
    activate: u32,
    height: u32,
    width: u32,
    accel_flags: u32,
    pixclock: u32,
    left_margin: u32,
    right_margin: u32,
    upper_margin: u32,
    lower_margin: u32,
    hsync_len: u32,
    vsync_len: u32,
    sync: u32,
    vmode: u32,
    rotate: u32,
    colorspace: u32,
    reserved: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct FbFixScreeninfo {
    id: [u8; 16],
    smem_start: u64,
    smem_len: u32,
    type_: u32,
    type_aux: u32,
    visual: u32,
    xpanstep: u16,
    ypanstep: u16,
    ywrapstep: u16,
    line_length: u32,
    mmio_start: u64,
    mmio_len: u32,
    accel: u32,
    capabilities: u16,
    reserved: [u16; 2],
}

pub fn open(_flags: u32) -> Result<Arc<dyn FileOps>> {
    if super::fb::info().is_none() {
        return Err(ENODEV);
    }
    Ok(Arc::new(FbDev))
}

impl FileOps for FbDev {
    fn read(&self, f: &File, b: &mut [u8]) -> Result<usize> {
        let fb = super::fb::info().ok_or(ENODEV)?;
        let size = fb.size();
        let pos = f.pos() as usize;
        if pos >= size {
            return Ok(0);
        }
        let n = b.len().min(size - pos);
        unsafe { core::ptr::copy_nonoverlapping((fb.ptr() as *const u8).add(pos), b.as_mut_ptr(), n) };
        f.set_pos((pos + n) as u64);
        Ok(n)
    }
    fn write(&self, f: &File, b: &[u8]) -> Result<usize> {
        let fb = super::fb::info().ok_or(ENODEV)?;
        let size = fb.size();
        let pos = f.pos() as usize;
        if pos >= size {
            return Err(ENOSPC);
        }
        let n = b.len().min(size - pos);
        unsafe { core::ptr::copy_nonoverlapping(b.as_ptr(), (fb.ptr() as *mut u8).add(pos), n) };
        f.set_pos((pos + n) as u64);
        Ok(n)
    }
    fn lseek(&self, f: &File, off: i64, whence: u32) -> Result<u64> {
        let fb = super::fb::info().ok_or(ENODEV)?;
        let base = match whence {
            0 => 0i64,
            1 => f.pos() as i64,
            2 => fb.size() as i64,
            _ => return Err(EINVAL),
        };
        let np = base + off;
        if np < 0 {
            return Err(EINVAL);
        }
        f.set_pos(np as u64);
        Ok(np as u64)
    }
    fn ioctl(&self, _f: &File, cmd: u32, arg: usize) -> Result<usize> {
        let fb = super::fb::info().ok_or(ENODEV)?;
        match cmd {
            0x4600 => {
                // FBIOGET_VSCREENINFO
                let mut v = FbVarScreeninfo::default();
                v.xres = fb.width as u32;
                v.yres = fb.height as u32;
                v.xres_virtual = v.xres;
                v.yres_virtual = v.yres;
                v.bits_per_pixel = fb.bpp as u32;
                v.red = FbBitfield { offset: fb.red_shift as u32, length: 8, msb_right: 0 };
                v.green = FbBitfield { offset: fb.green_shift as u32, length: 8, msb_right: 0 };
                v.blue = FbBitfield { offset: fb.blue_shift as u32, length: 8, msb_right: 0 };
                v.transp = FbBitfield { offset: 24, length: 0, msb_right: 0 };
                v.height = 0xffff_ffff;
                v.width = 0xffff_ffff;
                let bytes = unsafe { core::slice::from_raw_parts(&v as *const _ as *const u8, core::mem::size_of::<FbVarScreeninfo>()) };
                copy_to_user(arg, bytes)?;
                Ok(0)
            }
            0x4601 => Ok(0), // FBIOPUT_VSCREENINFO: accept, no change
            0x4602 => {
                let mut f = FbFixScreeninfo {
                    id: [0; 16],
                    smem_start: fb.phys,
                    smem_len: fb.size() as u32,
                    type_: 0,
                    type_aux: 0,
                    visual: 2, // FB_VISUAL_TRUECOLOR
                    xpanstep: 0,
                    ypanstep: 0,
                    ywrapstep: 0,
                    line_length: fb.pitch as u32,
                    mmio_start: 0,
                    mmio_len: 0,
                    accel: 0,
                    capabilities: 0,
                    reserved: [0; 2],
                };
                f.id[..8].copy_from_slice(b"mindosfb");
                let bytes = unsafe { core::slice::from_raw_parts(&f as *const _ as *const u8, core::mem::size_of::<FbFixScreeninfo>()) };
                copy_to_user(arg, bytes)?;
                Ok(0)
            }
            0x4606 => Ok(0), // FBIOPAN_DISPLAY
            0x4611 => Ok(0), // FBIOBLANK
            _ => Err(ENOTTY),
        }
    }
    fn mmap(&self, _file: &File, _prot: u32, _shared: bool) -> Result<Backing> {
        let fb = super::fb::info().ok_or(ENODEV)?;
        Ok(Backing::Phys { base: fb.phys & !0xfff, wc: true })
    }
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
}
