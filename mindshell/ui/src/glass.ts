// Frosted glass for windows that cannot see the wallpaper.
//
// Panels, popups and app windows are separate WebKit views, so CSS
// `backdrop-filter` only sees their own (transparent) page. A glass host gets
// a `.glass-bd` child instead: the blurred wallpaper, sized to the output and
// shifted by the host's position on it, so the crop under the window shows
// through. The built-in aurora is a gradient and stays sharp; image
// wallpapers are blurred once on a small canvas and cached as a data URL.

import { h } from './dom';
import { store } from './state';
import { thumbUrl } from './apps/shared';

export interface GlassHandle {
  el: HTMLElement;
  /** Re-read the wallpaper and the host's position. Cheap; call on every relayout. */
  update: () => void;
  dispose: () => void;
}

export interface GlassOpts {
  /** The output the host is on (its size is the wallpaper's). */
  output: string;
  /** Where the host's top-left corner sits on that output, in logical px. */
  origin: () => { x: number; y: number };
}

/** Width of the blur canvas; the result is stretched over the whole output. */
const BLUR_W = 480;
const cache = new Map<string, Promise<string>>();

function load(url: string): Promise<HTMLImageElement> {
  return new Promise((resolve, reject) => {
    const img = new Image();
    img.onload = () => resolve(img);
    img.onerror = () => reject(new Error(`image failed: ${url}`));
    img.src = url;
  });
}

/** A heavily blurred copy of the wallpaper as a data URL, cropped like `background-size: cover`. */
export function blurredWallpaper(path: string, aspect: number): Promise<string> {
  const key = `${path}@${aspect.toFixed(3)}`;
  let p = cache.get(key);
  if (p) return p;
  const url = /^[a-z]+:\/\//i.test(path) ? path : `file://${path}`;
  p = load(thumbUrl(path, BLUR_W * 2))
    .catch(() => load(url))
    .then((img) => {
      const w = BLUR_W;
      const hh = Math.max(1, Math.round(w / aspect));
      const big = document.createElement('canvas');
      big.width = w;
      big.height = hh;
      const g = big.getContext('2d')!;
      // cover-crop the source like the desktop does
      const ia = img.naturalWidth / img.naturalHeight;
      let sx = 0;
      let sy = 0;
      let sw = img.naturalWidth;
      let sh = img.naturalHeight;
      if (ia > aspect) {
        sw = Math.round(img.naturalHeight * aspect);
        sx = Math.round((img.naturalWidth - sw) / 2);
      } else {
        sh = Math.round(img.naturalWidth / aspect);
        sy = Math.round((img.naturalHeight - sh) / 2);
      }
      g.imageSmoothingEnabled = true;
      g.imageSmoothingQuality = 'high';
      g.drawImage(img, sx, sy, sw, sh, 0, 0, w, hh);
      // Blur by bouncing through a tiny canvas: bilinear resampling both ways
      // is a cheap wide gaussian, and it works in every canvas implementation.
      const small = document.createElement('canvas');
      small.width = Math.max(1, Math.round(w / 24));
      small.height = Math.max(1, Math.round(hh / 24));
      const sg = small.getContext('2d')!;
      sg.imageSmoothingEnabled = true;
      sg.imageSmoothingQuality = 'high';
      for (let i = 0; i < 2; i++) {
        sg.clearRect(0, 0, small.width, small.height);
        sg.drawImage(big, 0, 0, small.width, small.height);
        g.clearRect(0, 0, w, hh);
        g.drawImage(small, 0, 0, w, hh);
      }
      // a touch more saturation, the way frosted glass looks over a photo
      g.globalCompositeOperation = 'saturation';
      g.fillStyle = 'hsl(0 100% 50%)';
      g.globalAlpha = 0.25;
      g.fillRect(0, 0, w, hh);
      g.globalAlpha = 1;
      g.globalCompositeOperation = 'source-over';
      return big.toDataURL('image/jpeg', 0.82);
    });
  cache.set(key, p);
  p.catch(() => cache.delete(key));
  return p;
}

/** Mount a glass layer into `host` (which must be positioned with `isolation: isolate`). */
export function glassLayer(host: HTMLElement, opts: GlassOpts): GlassHandle {
  const el = h('div', { class: 'glass-bd' });
  host.prepend(el);
  let imageKey = '';
  let disposed = false;

  const update = () => {
    if (disposed) return;
    const out = store.output(opts.output);
    const w = out?.width ?? 1920;
    const hh = out?.height ?? 1080;
    const o = opts.origin();
    el.style.backgroundSize = `${w}px ${hh}px`;
    el.style.backgroundPosition = `${-Math.round(o.x)}px ${-Math.round(o.y)}px`;
    const wp = store.state.layout.desktop.wallpaper;
    const path = wp.mode === 'image' && wp.path ? wp.path : '';
    el.classList.toggle('builtin', !path);
    if (!path) {
      el.style.backgroundImage = '';
      imageKey = '';
      return;
    }
    const key = `${path}@${w}x${hh}`;
    if (key === imageKey) return;
    imageKey = key;
    blurredWallpaper(path, w / hh)
      .then((data) => {
        if (!disposed && imageKey === key) el.style.backgroundImage = `url("${data}")`;
      })
      .catch(() => {
        // no image: the tint over --bg-0 is still a fine glass
        if (!disposed && imageKey === key) el.style.backgroundImage = '';
      });
  };
  const offs = [store.on('layout', update), store.on('outputs', update)];
  update();
  return {
    el,
    update,
    dispose() {
      disposed = true;
      offs.forEach((f) => f());
      el.remove();
    },
  };
}
