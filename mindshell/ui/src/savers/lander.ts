// Lander: bring the craft down on a flat pad with the fuel it has. The
// autopilot lines up over the pad, tips into the drift to kill it, and holds
// a descent rate proportional to the height left.

import { backdrop, banner, C, clamp, rand, run, scoreLine } from './engine';
import type { Saver } from './types';

interface Pad {
  x0: number;
  x1: number;
  y: number;
  bonus: number;
}

const GRAVITY = 42;
const THRUST = 118;

export const lander: Saver = {
  id: 'lander',
  name: 'Lander',
  description: 'Setting a craft down on a flat pad, one landing after another.',
  start(canvas) {
    let w = 0;
    let h = 0;
    let ground: number[] = [];
    let pads: Pad[] = [];
    let stars: { x: number; y: number; a: number }[] = [];
    let x = 0;
    let y = 0;
    let vx = 0;
    let vy = 0;
    let angle = 0;
    let fuel = 1;
    let burn = 0;
    let score = 0;
    let flights = 0;
    let over = 0;
    let overText = '';
    let overColour = C.green;
    const bits: { x: number; y: number; vx: number; vy: number; life: number }[] = [];

    const heightAt = (px: number): number => {
      const i = clamp(Math.floor(px), 0, ground.length - 2);
      const k = px - i;
      return ground[i] * (1 - k) + ground[i + 1] * k;
    };

    const makeGround = (): void => {
      const base = h * 0.78;
      ground = new Array(Math.max(2, Math.round(w)));
      let level = base;
      let slope = 0;
      for (let i = 0; i < ground.length; i++) {
        slope += rand(-0.06, 0.06);
        slope = clamp(slope, -0.5, 0.5);
        level = clamp(level + slope, h * 0.55, h - 30);
        ground[i] = level;
      }
      pads = [];
      const count = 2 + Math.floor(Math.random() * 2);
      const band = w / count;
      for (let p = 0; p < count; p++) {
        const width = clamp(w * 0.06, 56, 120);
        const x0 = Math.round(band * p + rand(width * 0.4, band - width * 1.4));
        const x1 = Math.round(x0 + width);
        const flat = heightAt(x0);
        for (let i = x0; i <= x1 && i < ground.length; i++) ground[i] = flat;
        pads.push({ x0, x1, y: flat, bonus: Math.round(clamp(260 - width, 100, 220)) });
      }
    };

    const launch = (): void => {
      makeGround();
      x = rand(w * 0.15, w * 0.85);
      y = h * 0.1;
      vx = rand(-40, 40);
      vy = rand(0, 20);
      angle = 0;
      fuel = 1;
      burn = 0;
    };

    const reset = (nw: number, nh: number): void => {
      w = nw;
      h = nh;
      stars = Array.from({ length: 90 }, () => ({ x: rand(0, w), y: rand(0, h * 0.7), a: rand(0.15, 0.8) }));
      score = 0;
      flights = 0;
      launch();
    };

    const target = (): Pad => {
      let best = pads[0];
      let bestDistance = Infinity;
      for (const p of pads) {
        const d = Math.abs((p.x0 + p.x1) / 2 - x);
        if (d < bestDistance) {
          bestDistance = d;
          best = p;
        }
      }
      return best;
    };

    const explode = (): void => {
      for (let i = 0; i < 26; i++) {
        const a = rand(0, Math.PI * 2);
        bits.push({ x, y, vx: Math.cos(a) * rand(30, 220), vy: Math.sin(a) * rand(30, 200) - 40, life: rand(0.4, 1.1) });
      }
    };

    return run(
      canvas,
      (f) => {
        backdrop(f);

        f.g.save();
        for (const s of stars) {
          f.g.globalAlpha = s.a * (0.6 + 0.4 * Math.sin(f.t * 1.6 + s.x));
          f.g.fillStyle = '#cfe6f5';
          f.g.fillRect(s.x, s.y, 1.6, 1.6);
        }
        f.g.restore();

        if (over > 0) {
          over -= f.dt;
          if (over <= 0) launch();
        } else {
          const pad = target();
          const padX = (pad.x0 + pad.x1) / 2;
          const altitude = pad.y - y;

          // Sideways: aim for a drift that shrinks as the pad gets closer,
          // and tip the craft into whatever correction that needs.
          const wantVx = clamp((padX - x) * 0.45, -70, 70) * clamp(altitude / 120, 0.15, 1);
          const tilt = clamp((wantVx - vx) * 0.02, -0.55, 0.55);
          const wantAngle = altitude < 46 ? 0 : tilt;
          angle += clamp(wantAngle - angle, -2.4 * f.dt, 2.4 * f.dt);

          // Down: hold a descent rate that eases off near the ground.
          const wantVy = clamp(altitude * 0.42, 12, 70);
          const lined = Math.abs(x - padX) < (pad.x1 - pad.x0) * 0.35;
          burn = fuel > 0 && (vy > wantVy || (!lined && Math.abs(vx - wantVx) > 8 && altitude > 60)) ? 1 : 0;
          if (burn) {
            fuel = Math.max(0, fuel - f.dt * 0.06);
            vx += Math.sin(angle) * THRUST * f.dt;
            vy -= Math.cos(angle) * THRUST * f.dt;
          }
          vy += GRAVITY * f.dt;
          x += vx * f.dt;
          y += vy * f.dt;
          if (x < 10) {
            x = 10;
            vx = Math.abs(vx) * 0.4;
          }
          if (x > f.w - 10) {
            x = f.w - 10;
            vx = -Math.abs(vx) * 0.4;
          }

          const floor = heightAt(x);
          if (y + 12 >= floor) {
            const onPad = pads.some((p) => x > p.x0 && x < p.x1 && Math.abs(p.y - floor) < 2);
            const gentle = vy < 58 && Math.abs(vx) < 26 && Math.abs(angle) < 0.22;
            flights += 1;
            if (onPad && gentle) {
              score += 200 + Math.round(fuel * 300);
              over = 2.2;
              overText = 'TOUCHDOWN';
              overColour = C.green;
              y = floor - 12;
              vx = 0;
              vy = 0;
            } else {
              explode();
              over = 2;
              overText = onPad ? 'TOO FAST' : 'OFF THE PAD';
              overColour = C.pink;
            }
          }
        }

        // The ground.
        f.g.save();
        f.g.beginPath();
        f.g.moveTo(0, f.h);
        for (let i = 0; i < ground.length; i += 2) f.g.lineTo(i, ground[i]);
        f.g.lineTo(f.w, ground[ground.length - 1]);
        f.g.lineTo(f.w, f.h);
        f.g.closePath();
        f.g.fillStyle = '#0b1119';
        f.g.fill();
        f.g.strokeStyle = C.dim;
        f.g.lineWidth = 1.6;
        f.g.shadowColor = C.cyan;
        f.g.shadowBlur = 6;
        f.g.stroke();
        for (const p of pads) {
          f.g.strokeStyle = C.green;
          f.g.shadowColor = C.green;
          f.g.shadowBlur = 16;
          f.g.lineWidth = 3;
          f.g.beginPath();
          f.g.moveTo(p.x0, p.y);
          f.g.lineTo(p.x1, p.y);
          f.g.stroke();
          f.g.font = "600 10px 'JetBrains Mono', monospace";
          f.g.fillStyle = C.green;
          f.g.textAlign = 'center';
          f.g.fillText(`+${p.bonus}`, (p.x0 + p.x1) / 2, p.y + 16);
        }
        f.g.restore();

        // The craft.
        if (over <= 0 || overText === 'TOUCHDOWN') {
          f.g.save();
          f.g.translate(x, y);
          f.g.rotate(angle);
          f.g.shadowColor = C.cyan;
          f.g.shadowBlur = 14;
          f.g.strokeStyle = C.cyan;
          f.g.lineWidth = 2;
          f.g.beginPath();
          f.g.moveTo(0, -12);
          f.g.lineTo(9, 2);
          f.g.lineTo(-9, 2);
          f.g.closePath();
          f.g.stroke();
          f.g.beginPath();
          f.g.moveTo(-7, 2);
          f.g.lineTo(-11, 11);
          f.g.moveTo(7, 2);
          f.g.lineTo(11, 11);
          f.g.stroke();
          if (burn) {
            f.g.strokeStyle = C.amber;
            f.g.shadowColor = C.amber;
            f.g.beginPath();
            f.g.moveTo(-4, 3);
            f.g.lineTo(0, 12 + Math.random() * 10);
            f.g.lineTo(4, 3);
            f.g.stroke();
          }
          f.g.restore();
        }

        for (let i = bits.length - 1; i >= 0; i--) {
          const b = bits[i];
          b.life -= f.dt;
          if (b.life <= 0) {
            bits.splice(i, 1);
            continue;
          }
          b.vy += GRAVITY * f.dt;
          b.x += b.vx * f.dt;
          b.y += b.vy * f.dt;
          f.g.globalAlpha = Math.min(1, b.life);
          f.g.fillStyle = C.amber;
          f.g.fillRect(b.x, b.y, 2.4, 2.4);
        }
        f.g.globalAlpha = 1;

        // The instruments.
        const barW = clamp(f.w * 0.16, 120, 240);
        const barX = f.w / 2 - barW / 2;
        const barY = clamp(f.h * 0.04, 18, 40);
        f.g.save();
        f.g.strokeStyle = C.line;
        f.g.lineWidth = 1;
        f.g.strokeRect(barX, barY, barW, 8);
        f.g.fillStyle = fuel > 0.25 ? C.cyan : C.pink;
        f.g.shadowColor = f.g.fillStyle;
        f.g.shadowBlur = 12;
        f.g.fillRect(barX + 1, barY + 1, (barW - 2) * fuel, 6);
        f.g.font = "600 10px 'JetBrains Mono', monospace";
        f.g.fillStyle = C.dim;
        f.g.shadowBlur = 0;
        f.g.textAlign = 'center';
        f.g.fillText('FUEL', f.w / 2, barY + 22);
        f.g.restore();

        scoreLine(f, `SCORE ${score}`, `FLIGHT ${flights + 1}   ${Math.abs(Math.round(vy))} m/s`);
        if (over > 0) banner(f, overText, overColour, Math.min(1, over));
      },
      reset,
    );
  },
};
