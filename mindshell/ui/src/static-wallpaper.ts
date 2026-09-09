// The built-in wallpaper, pre-blurred. One small bitmap per palette, made
// once and cached: no animation, timers or compositor filters, so it costs
// nothing on a software-rendered desktop.
const cache = new Map<string, string>();

export function frostedWallpaper(light: boolean, background = light ? '#f4f5f7' : '#16181c', accent = '#35bf5c'): string {
  const key = `${light}|${background}|${accent}`;
  const saved = cache.get(key);
  if (saved) return saved;
  const big = document.createElement('canvas'); big.width = 480; big.height = 300;
  const ctx = big.getContext('2d');
  if (!ctx) return 'none';
  const shade = (t: number) => blend(background, light ? '#ffffff' : '#000000', t);
  const gradient = ctx.createLinearGradient(0, 0, 480, 300);
  gradient.addColorStop(0, shade(-.22));
  gradient.addColorStop(.55, background);
  gradient.addColorStop(1, shade(.35));
  ctx.fillStyle = gradient; ctx.fillRect(0, 0, 480, 300);
  // One soft pool of the highlight colour in the lower right, and a very
  // faint diagonal grain: enough to keep large flat areas from banding.
  const glow = ctx.createRadialGradient(370, 250, 10, 370, 250, 320);
  glow.addColorStop(0, rgba(accent, light ? .16 : .2));
  glow.addColorStop(1, rgba(accent, 0));
  ctx.fillStyle = glow; ctx.fillRect(0, 0, 480, 300);
  ctx.strokeStyle = light ? 'rgba(0,0,0,.028)' : 'rgba(255,255,255,.035)'; ctx.lineWidth = 1;
  for (let x = -240; x < 960; x += 107) {
    ctx.beginPath(); ctx.moveTo(x, 0); ctx.lineTo(x - 140, 300); ctx.stroke();
  }
  const small = document.createElement('canvas'); small.width = 30; small.height = 19;
  const low = small.getContext('2d');
  if (low) {
    low.imageSmoothingEnabled = ctx.imageSmoothingEnabled = true;
    for (let i = 0; i < 2; i++) {
      low.drawImage(big, 0, 0, 30, 19); ctx.drawImage(small, 0, 0, 480, 300);
    }
  }
  const result = `url("${big.toDataURL('image/png')}")`;
  cache.set(key, result);
  return result;
}

function parse(hex: string): [number, number, number] {
  const n = hex.replace('#', '');
  return [0, 2, 4].map((i) => parseInt(n.slice(i, i + 2), 16) || 0) as [number, number, number];
}

/** Negative `t` lightens towards `to`'s opposite; positive blends towards it. */
function blend(from: string, to: string, t: number): string {
  const a = parse(from); const b = t < 0 ? parse(to).map((v) => 255 - v) as [number, number, number] : parse(to);
  const k = Math.abs(t);
  return '#' + [0, 1, 2].map((i) => Math.max(0, Math.min(255, Math.round(a[i] + (b[i] - a[i]) * k))).toString(16).padStart(2, '0')).join('');
}

function rgba(hex: string, alpha: number): string {
  const [r, g, b] = parse(hex);
  return `rgba(${r},${g},${b},${alpha})`;
}
