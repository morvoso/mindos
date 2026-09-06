//! Linear framebuffer provided by the bootloader.

use crate::boot::limine::FbInfo;
use crate::sync::Once;

static FB: Once<Option<FbInfo>> = Once::new();

pub fn init(fb: Option<FbInfo>) {
    FB.call_once(|| fb);
}

pub fn info() -> Option<FbInfo> {
    FB.get().copied().flatten()
}

impl FbInfo {
    #[inline]
    pub fn ptr(&self) -> *mut u8 {
        self.virt as *mut u8
    }
    #[inline]
    pub fn size(&self) -> usize {
        self.pitch as usize * self.height as usize
    }
    #[inline]
    pub fn pixel(&self, r: u8, g: u8, b: u8) -> u32 {
        ((r as u32) << self.red_shift) | ((g as u32) << self.green_shift) | ((b as u32) << self.blue_shift)
    }
    pub fn fill_rect(&self, x: u32, y: u32, w: u32, h: u32, color: u32) {
        let x1 = (x + w).min(self.width);
        let y1 = (y + h).min(self.height);
        for yy in y..y1 {
            let row = unsafe { self.ptr().add(yy as usize * self.pitch as usize) as *mut u32 };
            for xx in x..x1 {
                unsafe { row.add(xx as usize).write_volatile(color) };
            }
        }
    }
}
