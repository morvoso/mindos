// Drift: a ship among rocks. The autopilot picks the nearest rock, leads it,
// and fires; when something gets too close it thrusts the other way. Rocks
// break into smaller rocks, and a cleared field brings a bigger wave.

import { backdrop, banner, C, rand, run, scoreLine } from './engine';
import type { Saver } from './types';

interface Rock {
  x: number;
  y: number;
  vx: number;
  vy: number;
  r: number;
  tier: number;
  spin: number;
  angle: number;
  shape: number[];
}

interface Shot {
  x: number;
  y: number;
  vx: number;
  vy: number;
  life: number;
}

interface Bit {
  x: number;
  y: number;
  vx: number;
  vy: number;
  life: number;
}

const SHOT_SPEED = 620;

export const drift: Saver = {
  id: 'drift',
  name: 'Drift',
  description: 'A ship, a field of rocks and an autopilot with good aim.',
  start(canvas) {
    let w = 0;
    let h = 0;
    let rocks: Rock[] = [];
    let shots: Shot[] = [];
    let bits: Bit[] = [];
    let x = 0;
    let y = 0;
    let vx = 0;
    let vy = 0;
    let angle = 0;
    let cool = 0;
    let thrust = 0;
    let dead = 0;
    let safe = 0;
    let score = 0;
    let wave = 1;
    let cleared = 0;

    const shapeOf = (): number[] => Array.from({ length: 11 }, () => rand(0.72, 1.24));

    const addRock = (rx: number, ry: number, tier: number): void => {
      const r = tier === 3 ? rand(42, 62) : tier === 2 ? rand(24, 34) : rand(12, 18);
      const a = rand(0, Math.PI * 2);
      const speed = rand(24, 70) * (4 - tier) * 0.6;
      rocks.push({ x: rx, y: ry, vx: Math.cos(a) * speed, vy: Math.sin(a) * speed, r, tier, spin: rand(-1.2, 1.2), angle: rand(0, 6.3), shape: shapeOf() });
    };

    const newWave = (): void => {
      rocks = [];
      const count = Math.min(4 + wave, 11);
      for (let i = 0; i < count; i++) {
        // Never right on top of the ship.
        let px = 0;
        let py = 0;
        do {
          px = rand(0, w);
          py = rand(0, h);
        } while (Math.hypot(px - w / 2, py - h / 2) < Math.min(w, h) * 0.3);
        addRock(px, py, 3);
      }
    };

    const respawn = (): void => {
      x = w / 2;
      y = h / 2;
      vx = 0;
      vy = 0;
      safe = 2;
    };

    const reset = (nw: number, nh: number): void => {
      w = nw;
      h = nh;
      shots = [];
      bits = [];
      score = 0;
      wave = 1;
      respawn();
      newWave();
    };

    const wrap = (p: { x: number; y: number }): void => {
      if (p.x < -40) p.x += w + 80;
      if (p.x > w + 40) p.x -= w + 80;
      if (p.y < -40) p.y += h + 80;
      if (p.y > h + 40) p.y -= h + 80;
    };

    const explode = (px: number, py: number, count: number): void => {
      for (let i = 0; i < count; i++) {
        const a = rand(0, Math.PI * 2);
        const s = rand(40, 260);
        bits.push({ x: px, y: py, vx: Math.cos(a) * s, vy: Math.sin(a) * s, life: rand(0.3, 0.9) });
      }
    };

    return run(
      canvas,
      (f) => {
        backdrop(f, 0.5);

        // ---- the autopilot ----------------------------------------------
        if (dead <= 0) {
          let target: Rock | null = null;
          let targetDistance = Infinity;
          let threat: Rock | null = null;
          let threatDistance = Infinity;
          for (const r of rocks) {
            const d = Math.hypot(r.x - x, r.y - y);
            if (d - r.r < threatDistance) {
              threatDistance = d - r.r;
              threat = r;
            }
            // Lead the rock: where it will be when a shot gets there.
            const flight = d / SHOT_SPEED;
            const px = r.x + r.vx * flight;
            const py = r.y + r.vy * flight;
            const lead = Math.hypot(px - x, py - y);
            if (lead < targetDistance) {
              targetDistance = lead;
              target = r;
            }
          }

          let want = angle;
          if (threat && threatDistance < 130) {
            // Too close: point away and burn.
            want = Math.atan2(y - threat.y, x - threat.x);
            thrust = 1;
          } else if (target) {
            const flight = Math.hypot(target.x - x, target.y - y) / SHOT_SPEED;
            want = Math.atan2(target.y + target.vy * flight - y, target.x + target.vx * flight - x);
            const drifting = Math.hypot(vx, vy);
            thrust = drifting < 40 && targetDistance > 320 ? 1 : 0;
          }
          let diff = ((want - angle + Math.PI * 3) % (Math.PI * 2)) - Math.PI;
          angle += Math.max(-4.2 * f.dt, Math.min(4.2 * f.dt, diff));
          diff = ((want - angle + Math.PI * 3) % (Math.PI * 2)) - Math.PI;

          cool -= f.dt;
          if (cool <= 0 && Math.abs(diff) < 0.12 && rocks.length) {
            cool = 0.28;
            shots.push({ x: x + Math.cos(angle) * 18, y: y + Math.sin(angle) * 18, vx: Math.cos(angle) * SHOT_SPEED + vx, vy: Math.sin(angle) * SHOT_SPEED + vy, life: 1.1 });
          }
          if (thrust) {
            vx += Math.cos(angle) * 260 * f.dt;
            vy += Math.sin(angle) * 260 * f.dt;
          }
          // Space is not quite empty here; the ship settles instead of
          // building up speed for ever.
          vx *= 1 - 0.5 * f.dt;
          vy *= 1 - 0.5 * f.dt;
          x += vx * f.dt;
          y += vy * f.dt;
          const ship = { x, y };
          wrap(ship);
          x = ship.x;
          y = ship.y;
          safe = Math.max(0, safe - f.dt);
        } else {
          dead -= f.dt;
          if (dead <= 0) respawn();
        }

        // ---- the rocks ---------------------------------------------------
        for (const r of rocks) {
          r.x += r.vx * f.dt;
          r.y += r.vy * f.dt;
          r.angle += r.spin * f.dt;
          wrap(r);
        }

        // ---- the shots ---------------------------------------------------
        for (let i = shots.length - 1; i >= 0; i--) {
          const s = shots[i];
          s.life -= f.dt;
          s.x += s.vx * f.dt;
          s.y += s.vy * f.dt;
          wrap(s);
          if (s.life <= 0) {
            shots.splice(i, 1);
            continue;
          }
          for (let j = rocks.length - 1; j >= 0; j--) {
            const r = rocks[j];
            if (Math.hypot(r.x - s.x, r.y - s.y) > r.r) continue;
            shots.splice(i, 1);
            rocks.splice(j, 1);
            score += r.tier === 3 ? 20 : r.tier === 2 ? 50 : 100;
            explode(r.x, r.y, 10);
            if (r.tier > 1) {
              addRock(r.x, r.y, r.tier - 1);
              addRock(r.x, r.y, r.tier - 1);
            }
            break;
          }
        }

        if (!rocks.length && cleared <= 0) {
          cleared = 1.6;
        }
        if (cleared > 0) {
          cleared -= f.dt;
          if (cleared <= 0) {
            wave += 1;
            newWave();
          }
        }

        // ---- did anything hit the ship? ----------------------------------
        if (dead <= 0 && safe <= 0) {
          for (const r of rocks) {
            if (Math.hypot(r.x - x, r.y - y) < r.r + 11) {
              explode(x, y, 26);
              dead = 1.4;
              score = Math.max(0, score - 100);
              break;
            }
          }
        }

        // ---- draw ---------------------------------------------------------
        f.g.save();
        f.g.lineJoin = 'round';
        f.g.shadowColor = C.cyan;
        f.g.shadowBlur = 10;
        f.g.strokeStyle = '#9fd7e6';
        f.g.lineWidth = 1.6;
        for (const r of rocks) {
          f.g.beginPath();
          for (let i = 0; i < r.shape.length; i++) {
            const a = r.angle + (i / r.shape.length) * Math.PI * 2;
            const rr = r.r * r.shape[i];
            const px = r.x + Math.cos(a) * rr;
            const py = r.y + Math.sin(a) * rr;
            if (i === 0) f.g.moveTo(px, py);
            else f.g.lineTo(px, py);
          }
          f.g.closePath();
          f.g.stroke();
        }

        f.g.strokeStyle = C.cyan;
        f.g.lineWidth = 2;
        f.g.shadowBlur = 16;
        for (const s of shots) {
          f.g.beginPath();
          f.g.moveTo(s.x, s.y);
          f.g.lineTo(s.x - s.vx * 0.012, s.y - s.vy * 0.012);
          f.g.stroke();
        }

        for (let i = bits.length - 1; i >= 0; i--) {
          const b = bits[i];
          b.life -= f.dt;
          if (b.life <= 0) {
            bits.splice(i, 1);
            continue;
          }
          b.x += b.vx * f.dt;
          b.y += b.vy * f.dt;
          f.g.globalAlpha = Math.min(1, b.life * 1.6);
          f.g.fillStyle = C.amber;
          f.g.fillRect(b.x, b.y, 2, 2);
        }
        f.g.globalAlpha = 1;

        if (dead <= 0 && (safe <= 0 || Math.sin(f.t * 22) > 0)) {
          f.g.translate(x, y);
          f.g.rotate(angle);
          f.g.strokeStyle = C.cyan;
          f.g.shadowBlur = 20;
          f.g.lineWidth = 2;
          f.g.beginPath();
          f.g.moveTo(16, 0);
          f.g.lineTo(-11, 9);
          f.g.lineTo(-6, 0);
          f.g.lineTo(-11, -9);
          f.g.closePath();
          f.g.stroke();
          if (thrust) {
            f.g.strokeStyle = C.amber;
            f.g.beginPath();
            f.g.moveTo(-7, 4);
            f.g.lineTo(-15 - Math.random() * 8, 0);
            f.g.lineTo(-7, -4);
            f.g.stroke();
          }
        }
        f.g.restore();

        scoreLine(f, `SCORE ${score}`, `WAVE ${wave}`);
        if (cleared > 0) banner(f, 'FIELD CLEAR', C.green, Math.min(1, cleared));
      },
      reset,
    );
  },
};
