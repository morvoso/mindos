// Small, pre-blurred wallpaper bitmaps. Generated once per theme, with no
// animation, timers or compositor filters on software-rendered desktops.
const cache = new Map<boolean, string>();
export function frostedWallpaper(light: boolean): string {
  const saved = cache.get(light);
  if (saved) return saved;
  const big = document.createElement('canvas'); big.width = 480; big.height = 300;
  const ctx = big.getContext('2d');
  if (!ctx) return 'none';
  const gradient = ctx.createLinearGradient(0, 0, 480, 300);
  gradient.addColorStop(0, light ? '#e6e3de' : '#303134');
  gradient.addColorStop(.7, light ? '#dedbd6' : '#1c1d1f');
  gradient.addColorStop(1, light ? '#e6e3de' : '#252628');
  ctx.fillStyle = gradient; ctx.fillRect(0, 0, 480, 300);
  ctx.strokeStyle = 'rgba(255,255,255,.045)'; ctx.lineWidth = 1;
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
  cache.set(light, result);
  return result;
}
