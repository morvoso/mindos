// The bit every screensaver shares: a canvas sized to the window, a frame
// loop that hands out a time step, and the MindOS palette. Nothing here
// knows what is being drawn.

export interface Frame {
  g: CanvasRenderingContext2D;
  /** Size in CSS pixels; the context is already scaled for the display. */
  w: number;
  h: number;
  /** Seconds since the last frame, never more than a twentieth of a second
   *  so a stall does not teleport anything across the screen. */
  dt: number;
  /** Seconds since the saver started. */
  t: number;
}

/** The colours a saver may use: the shell's own, so the screensaver looks
 *  like the rest of MindOS and not like a demo from another machine. */
export const C = {
  void: '#05070a',
  line: '#223041',
  dim: '#55657a',
  fg: '#e6edf3',
  cyan: '#19e3ff',
  violet: '#a78bfa',
  green: '#3ddc97',
  amber: '#ffb454',
  pink: '#ff5d8f',
};

/**
 * Run `frame` on every display refresh until the returned function is called.
 * `reset` is called once at the start and again whenever the window changes
 * size, so a saver can lay itself out for the space it has.
 */
export function run(canvas: HTMLCanvasElement, frame: (f: Frame) => void, reset?: (w: number, h: number) => void): () => void {
  const g = canvas.getContext('2d', { alpha: false });
  if (!g) return () => undefined;
  let w = 0;
  let h = 0;
  let t = 0;
  let last = performance.now();
  let raf = 0;
  let stopped = false;

  const size = (): void => {
    const box = canvas.getBoundingClientRect();
    const nw = Math.max(1, Math.round(box.width || canvas.clientWidth || window.innerWidth));
    const nh = Math.max(1, Math.round(box.height || canvas.clientHeight || window.innerHeight));
    if (nw === w && nh === h) return;
    w = nw;
    h = nh;
    // Real pixels for a crisp line, capped so a 4K display does not cost four
    // times the work for a picture nobody is looking at closely.
    const ratio = Math.min(window.devicePixelRatio || 1, 2);
    canvas.width = Math.round(w * ratio);
    canvas.height = Math.round(h * ratio);
    g.setTransform(ratio, 0, 0, ratio, 0, 0);
    g.fillStyle = C.void;
    g.fillRect(0, 0, w, h);
    reset?.(w, h);
  };

  const observer = new ResizeObserver(() => size());
  observer.observe(canvas);
  size();

  const tick = (now: number): void => {
    if (stopped) return;
    raf = requestAnimationFrame(tick);
    const dt = Math.min(0.05, Math.max(0, (now - last) / 1000));
    last = now;
    t += dt;
    frame({ g, w, h, dt, t });
  };
  raf = requestAnimationFrame(tick);

  return () => {
    stopped = true;
    cancelAnimationFrame(raf);
    observer.disconnect();
  };
}

/** Paint the background. `fade` under 1 leaves a trail behind moving things. */
export function backdrop(f: Frame, fade = 1): void {
  f.g.globalAlpha = fade;
  f.g.fillStyle = C.void;
  f.g.fillRect(0, 0, f.w, f.h);
  f.g.globalAlpha = 1;
}

/** Draw with a glow, the way everything in the shell is lit. */
export function glow(g: CanvasRenderingContext2D, colour: string, blur: number, draw: () => void): void {
  g.save();
  g.shadowColor = colour;
  g.shadowBlur = blur;
  g.strokeStyle = colour;
  g.fillStyle = colour;
  draw();
  g.restore();
}

/** The score line every game draws across the top. */
export function scoreLine(f: Frame, left: string, right = '', colour = C.cyan): void {
  const size = Math.max(13, Math.min(22, f.h / 34));
  f.g.save();
  f.g.font = `700 ${size}px 'Orbitron', system-ui, sans-serif`;
  f.g.textBaseline = 'top';
  f.g.shadowColor = colour;
  f.g.shadowBlur = 14;
  f.g.fillStyle = colour;
  f.g.globalAlpha = 0.85;
  f.g.textAlign = 'left';
  f.g.fillText(left, size * 1.6, size * 1.4);
  if (right) {
    f.g.textAlign = 'right';
    f.g.fillText(right, f.w - size * 1.6, size * 1.4);
  }
  f.g.restore();
}

/** A title card, for the moment after a game ends and before it starts again. */
export function banner(f: Frame, text: string, colour = C.cyan, alpha = 1): void {
  const size = Math.max(20, Math.min(54, f.h / 12));
  f.g.save();
  f.g.globalAlpha = alpha;
  f.g.font = `900 ${size}px 'Orbitron', system-ui, sans-serif`;
  f.g.textAlign = 'center';
  f.g.textBaseline = 'middle';
  f.g.shadowColor = colour;
  f.g.shadowBlur = 26;
  f.g.fillStyle = colour;
  f.g.fillText(text, f.w / 2, f.h / 2);
  f.g.restore();
}

export function rand(lo: number, hi: number): number {
  return lo + Math.random() * (hi - lo);
}

export function clamp(v: number, lo: number, hi: number): number {
  return v < lo ? lo : v > hi ? hi : v;
}
