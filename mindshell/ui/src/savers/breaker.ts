// Breaker: a bat, a ball and a wall of blocks. The bat follows the ball down
// and puts a little spin on it, so the ball works its way across the wall
// instead of drilling one hole.

import { backdrop, banner, C, clamp, rand, run, scoreLine } from './engine';
import type { Saver } from './types';

interface Brick {
  x: number;
  y: number;
  w: number;
  h: number;
  colour: string;
  hits: number;
  flash: number;
}

const ROWS = 6;
const COLOURS = [C.pink, C.amber, C.green, C.cyan, C.cyan, C.violet];

export const breaker: Saver = {
  id: 'breaker',
  name: 'Breaker',
  description: 'A wall of blocks and a bat that never gets bored.',
  start(canvas) {
    let w = 0;
    let h = 0;
    let bricks: Brick[] = [];
    let batW = 130;
    let batH = 14;
    let batX = 0;
    let batY = 0;
    let bx = 0;
    let by = 0;
    let vx = 0;
    let vy = 0;
    let speed = 460;
    let score = 0;
    let lives = 3;
    let level = 1;
    let pause = 0;
    let over = 0;
    let overText = '';
    const sparks: { x: number; y: number; vx: number; vy: number; life: number; colour: string }[] = [];

    const launch = (): void => {
      bx = w / 2;
      by = h * 0.62;
      speed = 420 + level * 26;
      const angle = rand(-0.5, 0.5) - Math.PI / 2;
      vx = Math.cos(angle) * speed;
      vy = Math.sin(angle) * speed;
      pause = 0.6;
    };

    const build = (): void => {
      bricks = [];
      const cols = Math.max(6, Math.min(16, Math.round(w / 110)));
      const pad = Math.max(4, w * 0.004);
      const left = w * 0.07;
      const width = (w - left * 2 - pad * (cols - 1)) / cols;
      const height = clamp(h * 0.035, 16, 34);
      const top = h * 0.12;
      for (let r = 0; r < ROWS; r++) {
        for (let c = 0; c < cols; c++) {
          bricks.push({
            x: left + c * (width + pad),
            y: top + r * (height + pad),
            w: width,
            h: height,
            colour: COLOURS[r % COLOURS.length],
            hits: r < 2 ? 2 : 1,
            flash: 0,
          });
        }
      }
    };

    const reset = (nw: number, nh: number): void => {
      w = nw;
      h = nh;
      batW = clamp(w * 0.11, 70, 210);
      batH = clamp(h * 0.016, 10, 20);
      batY = h - clamp(h * 0.08, 40, 90);
      batX = w / 2;
      score = 0;
      lives = 3;
      level = 1;
      build();
      launch();
    };

    const burst = (x: number, y: number, colour: string): void => {
      for (let i = 0; i < 9; i++) {
        const a = rand(0, Math.PI * 2);
        sparks.push({ x, y, vx: Math.cos(a) * rand(40, 220), vy: Math.sin(a) * rand(40, 220), life: rand(0.25, 0.6), colour });
      }
    };

    return run(
      canvas,
      (f) => {
        backdrop(f, 0.42);

        if (over > 0) {
          over -= f.dt;
          if (over <= 0) {
            if (overText === 'CLEARED') level += 1;
            else {
              level = 1;
              score = 0;
              lives = 3;
            }
            build();
            launch();
          }
        } else if (pause > 0) {
          pause -= f.dt;
        } else {
          // The bat: sit under the ball while it comes down, drift back to
          // the middle while it is up in the wall.
          const want = vy > 0 ? bx + vx * 0.06 : f.w / 2 + (bx - f.w / 2) * 0.4;
          const top = f.w * 1.1;
          batX = clamp(batX + clamp(want - batX, -top * f.dt, top * f.dt), batW / 2, f.w - batW / 2);

          bx += vx * f.dt;
          by += vy * f.dt;
          const r = 8;
          if (bx < r && vx < 0) {
            bx = r;
            vx = -vx;
          }
          if (bx > f.w - r && vx > 0) {
            bx = f.w - r;
            vx = -vx;
          }
          if (by < r && vy < 0) {
            by = r;
            vy = -vy;
          }
          // The bat.
          if (vy > 0 && by + r >= batY && by + r < batY + batH + 14 && Math.abs(bx - batX) < batW / 2 + r) {
            by = batY - r;
            const offset = clamp((bx - batX) / (batW / 2), -1, 1);
            speed = Math.min(speed * 1.02, 900);
            const angle = -Math.PI / 2 + offset * 1.05;
            vx = Math.cos(angle) * speed;
            vy = Math.sin(angle) * speed;
          }
          // The wall.
          for (let i = 0; i < bricks.length; i++) {
            const b = bricks[i];
            if (bx + r < b.x || bx - r > b.x + b.w || by + r < b.y || by - r > b.y + b.h) continue;
            const fromSide = Math.abs(bx - (b.x + b.w / 2)) / b.w > Math.abs(by - (b.y + b.h / 2)) / b.h;
            if (fromSide) vx = -vx;
            else vy = -vy;
            b.hits -= 1;
            b.flash = 1;
            score += 10;
            burst(bx, by, b.colour);
            if (b.hits <= 0) bricks.splice(i, 1);
            break;
          }
          if (by > f.h + 40) {
            lives -= 1;
            if (lives <= 0) {
              over = 2;
              overText = 'GAME OVER';
            } else {
              launch();
            }
          }
          if (!bricks.length) {
            over = 1.8;
            overText = 'CLEARED';
          }
        }

        // The wall.
        f.g.save();
        for (const b of bricks) {
          b.flash = Math.max(0, b.flash - f.dt * 4);
          f.g.shadowColor = b.colour;
          f.g.shadowBlur = 10 + b.flash * 20;
          f.g.globalAlpha = b.hits > 1 ? 1 : 0.72;
          f.g.fillStyle = b.colour;
          f.g.beginPath();
          f.g.roundRect(b.x, b.y, b.w, b.h, 4);
          f.g.fill();
        }
        f.g.restore();

        // The sparks a block leaves behind.
        for (let i = sparks.length - 1; i >= 0; i--) {
          const s = sparks[i];
          s.life -= f.dt;
          if (s.life <= 0) {
            sparks.splice(i, 1);
            continue;
          }
          s.x += s.vx * f.dt;
          s.y += s.vy * f.dt;
          s.vy += 420 * f.dt;
          f.g.globalAlpha = Math.max(0, s.life * 1.6);
          f.g.fillStyle = s.colour;
          f.g.fillRect(s.x, s.y, 2.5, 2.5);
        }
        f.g.globalAlpha = 1;

        // The bat and the ball.
        f.g.save();
        f.g.shadowColor = C.cyan;
        f.g.shadowBlur = 18;
        f.g.fillStyle = C.cyan;
        f.g.beginPath();
        f.g.roundRect(batX - batW / 2, batY, batW, batH, batH / 2);
        f.g.fill();
        f.g.fillStyle = '#dffaff';
        f.g.beginPath();
        f.g.arc(bx, by, 8, 0, Math.PI * 2);
        f.g.fill();
        f.g.restore();

        scoreLine(f, `SCORE ${score}`, `${'●'.repeat(Math.max(0, lives))}  LEVEL ${level}`);
        if (over > 0) banner(f, overText, overText === 'CLEARED' ? C.green : C.pink, Math.min(1, over));
      },
      reset,
    );
  },
};
