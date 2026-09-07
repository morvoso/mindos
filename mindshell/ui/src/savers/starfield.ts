// Starfield: the calm one. Stars stream past, the field drifts, and every so
// often a comet crosses it. Nothing to win, nothing to watch for.

import { backdrop, C, rand, run } from './engine';
import type { Saver } from './types';

interface Star {
  x: number;
  y: number;
  z: number;
  hue: number;
}

interface Comet {
  x: number;
  y: number;
  vx: number;
  vy: number;
  life: number;
}

const COUNT = 420;
const DEPTH = 1000;

export const starfield: Saver = {
  id: 'starfield',
  name: 'Starfield',
  description: 'Stars streaming past, with the odd comet.',
  start(canvas) {
    const stars: Star[] = [];
    const comets: Comet[] = [];
    let cx = 0;
    let cy = 0;
    let speed = 220;

    const place = (s: Star, fresh: boolean): void => {
      s.x = rand(-1, 1);
      s.y = rand(-1, 1);
      s.z = fresh ? rand(1, DEPTH) : DEPTH;
      s.hue = Math.random();
    };

    const reset = (w: number, h: number): void => {
      cx = w / 2;
      cy = h / 2;
      stars.length = 0;
      for (let i = 0; i < COUNT; i++) {
        const s: Star = { x: 0, y: 0, z: 0, hue: 0 };
        place(s, true);
        stars.push(s);
      }
    };

    return run(
      canvas,
      (f) => {
        backdrop(f, 0.25); // the trails behind the stars
        const scale = Math.max(f.w, f.h);
        // The heading wanders, so the streaks never settle into one pattern.
        const driftX = Math.cos(f.t * 0.11) * 0.16;
        const driftY = Math.sin(f.t * 0.083) * 0.16;
        speed += (rand(150, 340) - speed) * f.dt * 0.05;

        f.g.lineCap = 'round';
        for (const s of stars) {
          const was = s.z;
          s.z -= speed * f.dt;
          s.x += driftX * f.dt;
          s.y += driftY * f.dt;
          if (s.z <= 1) {
            place(s, false);
            continue;
          }
          const k = scale / s.z;
          const px = cx + s.x * scale * k * 0.5;
          const py = cy + s.y * scale * k * 0.5;
          if (px < -60 || px > f.w + 60 || py < -60 || py > f.h + 60) {
            place(s, false);
            continue;
          }
          const kWas = scale / was;
          const qx = cx + s.x * scale * kWas * 0.5;
          const qy = cy + s.y * scale * kWas * 0.5;
          const near = 1 - s.z / DEPTH;
          const colour = s.hue > 0.9 ? C.violet : s.hue > 0.72 ? C.cyan : '#dfe9f5';
          f.g.globalAlpha = 0.35 + near * 0.65;
          f.g.strokeStyle = colour;
          f.g.lineWidth = 0.8 + near * 2.4;
          f.g.beginPath();
          f.g.moveTo(qx, qy);
          f.g.lineTo(px, py);
          f.g.stroke();
          // The star itself, so the field reads even when nothing is moving fast.
          f.g.fillStyle = colour;
          f.g.fillRect(px - 0.5, py - 0.5, 1 + near * 2, 1 + near * 2);
        }
        f.g.globalAlpha = 1;

        if (Math.random() < f.dt * 0.22) {
          const fromLeft = Math.random() < 0.5;
          comets.push({
            x: fromLeft ? -40 : f.w + 40,
            y: rand(0, f.h * 0.7),
            vx: (fromLeft ? 1 : -1) * rand(340, 620),
            vy: rand(60, 180),
            life: 1,
          });
        }
        for (let i = comets.length - 1; i >= 0; i--) {
          const c = comets[i];
          c.x += c.vx * f.dt;
          c.y += c.vy * f.dt;
          c.life -= f.dt * 0.22;
          if (c.life <= 0 || c.x < -120 || c.x > f.w + 120 || c.y > f.h + 120) {
            comets.splice(i, 1);
            continue;
          }
          f.g.save();
          f.g.globalAlpha = Math.min(1, c.life);
          f.g.shadowColor = C.cyan;
          f.g.shadowBlur = 18;
          f.g.strokeStyle = C.cyan;
          f.g.lineWidth = 2;
          f.g.beginPath();
          f.g.moveTo(c.x, c.y);
          f.g.lineTo(c.x - c.vx * 0.09, c.y - c.vy * 0.09);
          f.g.stroke();
          f.g.restore();
        }
      },
      reset,
    );
  },
};
