//! Renders a sample sheet through the compositor's own text stack, so the
//! effect of changing `TEXT_GAMMA` or a face in `src/text.rs` can be looked at
//! without booting the compositor:
//!
//! ```sh
//! cargo run --example textproof -- /tmp/text.ppm
//! ```
use mindwm::text::{hex, Canvas, Face, TextRenderer};

const SAMPLE: &str = "Handgloves 0123 — the quick brown fox; Illegal1 rn/m O0";

fn main() {
    let out = std::env::args().nth(1).expect("usage: textproof <out.ppm>");
    let (w, h) = (900, 470);
    let mut c = Canvas::new(w, h);
    c.fill_rect(0, 0, w, h, hex(0x0a0d12));
    let mut t = TextRenderer::new();
    let (fg, dim, accent) = (hex(0xe6edf3), hex(0x8b9bb0), hex(0x19e3ff));

    let mut y = 16;
    for (label, face, px) in [
        ("Body 15", Face::Body, 15.0),
        ("Body 17", Face::Body, 17.0),
        ("BodyBold 17", Face::BodyBold, 17.0),
        ("Label 13", Face::Label, 13.0),
        ("LabelBold 14", Face::LabelBold, 14.0),
        ("Mono 13", Face::Mono, 13.0),
        ("Mono 15", Face::Mono, 15.0),
    ] {
        t.draw(&mut c, 12, y, None, label, 11.0, dim, Face::Mono);
        t.draw(&mut c, 130, y, None, SAMPLE, px, fg, face);
        y += t.line_height(px, face).max(20) + 8;
    }

    y += 6;
    t.draw_spaced(&mut c, 130, y, "MIND · SYSTEM STATUS", 14.0, accent, Face::LabelBold, 3);
    y += 30;
    let copy = "Wrapped body copy at 16 px: gamma-corrected coverage keeps the stems from washing \
                out on a dark ground, and subpixel placement keeps the letter spacing even instead \
                of quantised to whole pixels.";
    t.draw(&mut c, 130, y, Some(720), copy, 16.0, fg, Face::Body);
    y += 76;
    t.draw(&mut c, 130, y, None, "◈ fallback glyph via DejaVu", 15.0, accent, Face::Body);
    y += 28;
    t.draw(&mut c, 130, y, None, "MINDOS", 34.0, fg, Face::Display);

    // A light plate, to exercise the dark-on-light coverage curve as well.
    c.fill_rect(560, y - 10, 320, 52, hex(0xe6edf3));
    t.draw(&mut c, 574, y, None, "Dark on light", 18.0, hex(0x0a0d12), Face::Body);

    let mut ppm = format!("P6\n{w} {h}\n255\n").into_bytes();
    for px in c.data.chunks(4) {
        ppm.extend_from_slice(&[px[2], px[1], px[0]]);
    }
    std::fs::write(out, ppm).unwrap();
}
