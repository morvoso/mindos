//! Renders a decorated window the way the compositor composites it (the
//! frame's shadow and border, the title bar, a client area) over an
//! aurora-like ground, so the look can be tuned without booting:
//!
//! ```sh
//! cargo run --example frameproof -- /tmp/frame.ppm
//! ```
use mindwm::shell::{
    frame::{preview, FrameStyle, SEAM_BOTTOM, SEAM_LEFT, SEAM_RIGHT, SEAM_TOP, SHADOW, TILE_SHADOW},
    ssd::{HeaderBar, HEADER_BAR_HEIGHT, RADIUS},
};
use mindwm::text::{alpha, hex, Canvas, Face, TextRenderer};

fn write_ppm(path: &str, canvas: &Canvas) {
    use std::io::Write;
    let mut out = std::fs::File::create(path).unwrap();
    write!(out, "P6\n{} {}\n255\n", canvas.width, canvas.height).unwrap();
    let mut buf = Vec::with_capacity((canvas.width * canvas.height * 3) as usize);
    for px in canvas.data.chunks_exact(4) {
        buf.extend_from_slice(&[px[2], px[1], px[0]]);
    }
    out.write_all(&buf).unwrap();
}

fn window(c: &mut Canvas, t: &mut TextRenderer, x: i32, y: i32, w: i32, h: i32, title: &str, focused: bool, tiled: bool, seams: u8) {
    let style = FrameStyle {
        focused,
        radius: RADIUS,
        shadow: if tiled { TILE_SHADOW } else { SHADOW },
        accent: mindwm::config::accent_rgb(),
        seams,
        seam: mindwm::config::seam_rgb(),
    };
    let (frame, m) = preview(style, 1, w, h);
    c.draw_canvas(x - m, y - m, &frame);
    let mut bar = HeaderBar::default();
    bar.set_title(title);
    bar.set_focused(focused);
    bar.set_tiled(tiled);
    bar.redraw(w, 1);
    c.draw_canvas(x, y, &bar.draw(1));
    // the client: an opaque dark app with some text
    c.fill_rect(x, y + HEADER_BAR_HEIGHT, w, h - HEADER_BAR_HEIGHT, hex(0x10151c));
    let mut yy = y + HEADER_BAR_HEIGHT + 18;
    for line in ["$ cargo build --release", "   Compiling mindwm v0.3.0", "    Finished `release` profile", "$ _"] {
        t.draw(c, x + 16, yy, None, line, 13.0, hex(0xc9d3de), Face::Mono);
        yy += 20;
    }
}

fn main() {
    let out = std::env::args().nth(1).expect("usage: frameproof <out.ppm>");
    let (w, h) = (1240, 700);
    let mut c = Canvas::new(w, h);
    // a rough aurora: navy ground with a cyan and a violet glow
    for yy in 0..h {
        for xx in 0..w {
            let fx = xx as f32 / w as f32;
            let fy = yy as f32 / h as f32;
            let base = [0.05 + 0.03 * (1.0 - fy), 0.07 + 0.03 * (1.0 - fy), 0.12 + 0.05 * (1.0 - fy), 1.0];
            let cyan = (1.0 - ((fx - 0.2).powi(2) + (fy - 0.25).powi(2)).sqrt() * 2.2).max(0.0) * 0.34;
            let violet = (1.0 - ((fx - 0.85).powi(2) + (fy - 0.8).powi(2)).sqrt() * 2.0).max(0.0) * 0.4;
            let col = [
                base[0] + cyan * 0.1 + violet * 0.65,
                base[1] + cyan * 0.89 + violet * 0.55,
                base[2] + cyan * 1.0 + violet * 0.98,
                1.0,
            ];
            c.blend(xx, yy, [col[0].min(1.0), col[1].min(1.0), col[2].min(1.0), 1.0]);
        }
    }
    let mut t = TextRenderer::new();
    // Left: floating windows, which touch nothing and keep the hairline ring.
    window(&mut c, &mut t, 40, 90, 460, 300, "Terminal — ~/src/mindos", false, false, 0);
    window(&mut c, &mut t, 190, 230, 420, 320, "Settings", true, false, 0);
    // Right: a tiling laid out the way dwindle would, one wide tile beside two
    // stacked ones. Only the sides that face another tile carry a seam; the
    // outer edges of the tiling are hairlines like anything else.
    window(&mut c, &mut t, 640, 70, 276, 520, "Firefox", false, true, SEAM_RIGHT);
    window(&mut c, &mut t, 924, 70, 276, 256, "Vesktop", true, true, SEAM_LEFT | SEAM_BOTTOM);
    window(&mut c, &mut t, 924, 334, 276, 256, "kitty", false, true, SEAM_LEFT | SEAM_TOP);
    t.draw(&mut c, 20, h - 30, None, "frameproof: floating (no seams) / three tiles, seams on the sides that touch", 12.0, alpha(hex(0xe6edf3), 0.6), Face::Label);
    write_ppm(&out, &c);
}
