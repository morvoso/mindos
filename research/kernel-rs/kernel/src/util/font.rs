//! PC Screen Font (PSF1/PSF2) glyph access for the kernel console.

static FONT_DATA: &[u8] = include_bytes!("../../assets/font8x16.psfu");

pub struct Font {
    glyphs: &'static [u8],
    pub width: usize,
    pub height: usize,
    charsize: usize,
    count: usize,
    /// codepoint (< 256) -> glyph index
    map: [u16; 256],
}

pub fn load() -> Font {
    let d = FONT_DATA;
    if d.len() >= 32 && d[0..4] == [0x72, 0xb5, 0x4a, 0x86] {
        let rd = |o: usize| u32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]]) as usize;
        let headersize = rd(8);
        let flags = rd(12);
        let length = rd(16);
        let charsize = rd(20);
        let height = rd(24);
        let width = rd(28);
        let glyphs = &d[headersize..headersize + length * charsize];
        let mut map = [0u16; 256];
        for i in 0..256usize.min(length) {
            map[i] = i as u16;
        }
        if flags & 1 != 0 {
            // unicode table: for each glyph, UTF-8 sequences terminated by 0xFF
            let mut p = headersize + length * charsize;
            let mut g = 0usize;
            while p < d.len() && g < length {
                let start = p;
                while p < d.len() && d[p] != 0xFF {
                    p += 1;
                }
                if let Ok(s) = core::str::from_utf8(&d[start..p]) {
                    for ch in s.chars() {
                        let c = ch as u32;
                        if c < 256 {
                            map[c as usize] = g as u16;
                        }
                    }
                }
                p += 1;
                g += 1;
            }
        }
        Font { glyphs, width, height, charsize, count: length, map }
    } else if d.len() >= 4 && d[0..2] == [0x36, 0x04] {
        let mode = d[2];
        let charsize = d[3] as usize;
        let count = if mode & 1 != 0 { 512 } else { 256 };
        let mut map = [0u16; 256];
        for (i, m) in map.iter_mut().enumerate() {
            *m = i as u16;
        }
        Font { glyphs: &d[4..4 + count * charsize], width: 8, height: charsize, charsize, count, map }
    } else {
        panic!("unsupported console font format");
    }
}

impl Font {
    /// Bitmap rows (MSB = leftmost pixel) for a character.
    pub fn glyph(&self, c: char) -> &[u8] {
        let cp = c as u32;
        let idx = if cp < 256 { self.map[cp as usize] as usize } else { self.map[b'?' as usize] as usize };
        let idx = idx.min(self.count - 1);
        &self.glyphs[idx * self.charsize..(idx + 1) * self.charsize]
    }
}
