// Wave: a formation of drones works its way down the screen while a cannon
// at the bottom picks them off. The cannon lines up the lowest drone in the
// nearest column, and sidesteps anything falling towards it. Four barriers
// wear away as the fight goes on.

import { backdrop, banner, C, clamp, rand, run, scoreLine } from './engine';
import type { Saver } from './types';

interface Drone {
  col: number;
  row: number;
  alive: boolean;
}

interface Shot {
  x: number;
  y: number;
  vy: number;
}

interface Block {
  x: number;
  y: number;
  s: number;
  hp: number;
}

const COLS = 9;
const ROWS = 4;

export const wave: Saver = {
  id: 'wave',
  name: 'Wave',
  description: 'Drones come down in formation; the cannon works up the columns.',
  start(canvas) {
    let w = 0;
    let h = 0;
    let drones: Drone[] = [];
    let blocks: Block[] = [];
    let shots: Shot[] = [];
    let bombs: Shot[] = [];
    let gapX = 70;
    let gapY = 52;
    let originX = 0;
    let originY = 0;
    let march = 1;
    let stepTimer = 0;
    let gunX = 0;
    let gunY = 0;
    let cool = 0;
    let score = 0;
    let round = 1;
    let over = 0;
    let overText = '';
    const bits: { x: number; y: number; vx: number; vy: number; life: number; colour: string }[] = [];

    const droneAt = (d: Drone): { x: number; y: number } => ({ x: originX + d.col * gapX, y: originY + d.row * gapY });

    const buildShields = (): void => {
      blocks = [];
      const cell = clamp(w * 0.012, 8, 18);
      const y = h - clamp(h * 0.24, 110, 240);
      for (let s = 0; s < 4; s++) {
        const left = w * (0.16 + s * 0.226);
        for (let r = 0; r < 3; r++) {
          for (let c = 0; c < 5; c++) {
            if (r === 2 && c > 1 && c < 3) continue; // the notch underneath
            blocks.push({ x: left + c * cell, y: y + r * cell, s: cell, hp: 3 });
          }
        }
      }
    };

    const newRound = (): void => {
      drones = [];
      for (let r = 0; r < ROWS; r++) {
        for (let c = 0; c < COLS; c++) drones.push({ col: c, row: r, alive: true });
      }
      originX = (w - (COLS - 1) * gapX) / 2;
      originY = h * 0.14;
      march = 1;
      shots = [];
      bombs = [];
      buildShields();
    };

    const reset = (nw: number, nh: number): void => {
      w = nw;
      h = nh;
      gapX = clamp(w / (COLS + 3), 44, 160);
      gapY = clamp(h / 12, 34, 88);
      gunY = h - clamp(h * 0.09, 44, 96);
      gunX = w / 2;
      score = 0;
      round = 1;
      newRound();
    };

    const living = (): Drone[] => drones.filter((d) => d.alive);

    const hitBlock = (x: number, y: number): boolean => {
      for (let i = 0; i < blocks.length; i++) {
        const b = blocks[i];
        if (x < b.x || x > b.x + b.s || y < b.y || y > b.y + b.s) continue;
        b.hp -= 1;
        if (b.hp <= 0) blocks.splice(i, 1);
        return true;
      }
      return false;
    };

    const burst = (x: number, y: number, colour: string): void => {
      for (let i = 0; i < 10; i++) {
        const a = rand(0, Math.PI * 2);
        bits.push({ x, y, vx: Math.cos(a) * rand(30, 200), vy: Math.sin(a) * rand(30, 200), life: rand(0.2, 0.6), colour });
      }
    };

    return run(
      canvas,
      (f) => {
        backdrop(f);

        if (over > 0) {
          over -= f.dt;
          if (over <= 0) {
            if (overText === 'WAVE CLEARED') round += 1;
            else {
              round = 1;
              score = 0;
            }
            newRound();
          }
        } else {
          const alive = living();
          // The formation steps sideways, and down a row at the edge. It
          // steps faster the fewer of them are left.
          stepTimer -= f.dt;
          const period = clamp(0.55 - round * 0.03 - (COLS * ROWS - alive.length) * 0.012, 0.09, 0.6);
          if (stepTimer <= 0) {
            stepTimer = period;
            const xs = alive.map((d) => droneAt(d).x);
            const leftEdge = Math.min(...xs);
            const rightEdge = Math.max(...xs);
            if ((march > 0 && rightEdge + gapX * 0.6 > f.w - 20) || (march < 0 && leftEdge - gapX * 0.6 < 20)) {
              march = -march;
              originY += gapY * 0.5;
            } else {
              originX += march * gapX * 0.34;
            }
            // Somebody drops a bomb.
            if (alive.length && Math.random() < 0.55) {
              const d = alive[Math.floor(Math.random() * alive.length)];
              const p = droneAt(d);
              bombs.push({ x: p.x, y: p.y + 14, vy: rand(180, 300) });
            }
          }

          // The cannon: line up the lowest drone in the nearest column, and
          // get out from under anything falling.
          let want = f.w / 2;
          let lowest: Drone | null = null;
          for (const d of alive) {
            if (!lowest || d.row > lowest.row || (d.row === lowest.row && Math.abs(droneAt(d).x - gunX) < Math.abs(droneAt(lowest).x - gunX))) lowest = d;
          }
          if (lowest) want = droneAt(lowest).x;
          for (const b of bombs) {
            if (Math.abs(b.x - gunX) < 26 && b.y > gunY - 320) want = gunX + (b.x < gunX ? 90 : -90);
          }
          const top = f.w * 0.6;
          gunX = clamp(gunX + clamp(want - gunX, -top * f.dt, top * f.dt), 30, f.w - 30);

          cool -= f.dt;
          if (cool <= 0 && lowest && Math.abs(droneAt(lowest).x - gunX) < 10) {
            cool = 0.34;
            shots.push({ x: gunX, y: gunY - 14, vy: -740 });
          }

          for (let i = shots.length - 1; i >= 0; i--) {
            const s = shots[i];
            s.y += s.vy * f.dt;
            if (s.y < -10 || hitBlock(s.x, s.y)) {
              shots.splice(i, 1);
              continue;
            }
            let hit = false;
            for (const d of alive) {
              const p = droneAt(d);
              if (Math.abs(p.x - s.x) > gapX * 0.32 || Math.abs(p.y - s.y) > gapY * 0.3) continue;
              d.alive = false;
              score += (ROWS - d.row) * 10;
              burst(p.x, p.y, C.violet);
              hit = true;
              break;
            }
            if (hit) shots.splice(i, 1);
          }

          for (let i = bombs.length - 1; i >= 0; i--) {
            const b = bombs[i];
            b.y += b.vy * f.dt;
            if (b.y > f.h + 10 || hitBlock(b.x, b.y)) {
              bombs.splice(i, 1);
              continue;
            }
            if (Math.abs(b.x - gunX) < 18 && Math.abs(b.y - gunY) < 16) {
              bombs.splice(i, 1);
              burst(gunX, gunY, C.pink);
              over = 2;
              overText = 'CANNON DOWN';
            }
          }

          if (!living().length) {
            over = 1.8;
            overText = 'WAVE CLEARED';
          } else if (originY + (ROWS - 1) * gapY > gunY - 40) {
            over = 2;
            overText = 'OVERRUN';
          }
        }

        // ---- draw ---------------------------------------------------------
        f.g.save();
        f.g.shadowBlur = 12;
        for (const d of drones) {
          if (!d.alive) continue;
          const p = droneAt(d);
          const size = gapX * 0.22;
          const colour = d.row === 0 ? C.pink : d.row === 1 ? C.amber : C.violet;
          f.g.shadowColor = colour;
          f.g.strokeStyle = colour;
          f.g.fillStyle = colour;
          f.g.lineWidth = 2;
          // A blunt hexagon with two feelers: nothing anybody has seen before.
          f.g.beginPath();
          f.g.moveTo(p.x - size, p.y);
          f.g.lineTo(p.x - size * 0.5, p.y - size * 0.8);
          f.g.lineTo(p.x + size * 0.5, p.y - size * 0.8);
          f.g.lineTo(p.x + size, p.y);
          f.g.lineTo(p.x + size * 0.45, p.y + size * 0.75);
          f.g.lineTo(p.x - size * 0.45, p.y + size * 0.75);
          f.g.closePath();
          f.g.stroke();
          const wiggle = Math.sin(f.t * 6 + d.col) * size * 0.25;
          f.g.beginPath();
          f.g.moveTo(p.x - size * 0.5, p.y - size * 0.8);
          f.g.lineTo(p.x - size * 0.8, p.y - size * 1.5 + wiggle);
          f.g.moveTo(p.x + size * 0.5, p.y - size * 0.8);
          f.g.lineTo(p.x + size * 0.8, p.y - size * 1.5 - wiggle);
          f.g.stroke();
          f.g.globalAlpha = 0.25;
          f.g.fill();
          f.g.globalAlpha = 1;
        }

        // The barriers.
        for (const b of blocks) {
          f.g.shadowColor = C.green;
          f.g.shadowBlur = 8;
          f.g.globalAlpha = 0.35 + b.hp * 0.2;
          f.g.fillStyle = C.green;
          f.g.fillRect(b.x, b.y, b.s - 1.5, b.s - 1.5);
        }
        f.g.globalAlpha = 1;

        f.g.shadowColor = C.cyan;
        f.g.shadowBlur = 16;
        f.g.strokeStyle = C.cyan;
        f.g.lineWidth = 2.5;
        for (const s of shots) {
          f.g.beginPath();
          f.g.moveTo(s.x, s.y);
          f.g.lineTo(s.x, s.y + 14);
          f.g.stroke();
        }
        f.g.strokeStyle = C.pink;
        f.g.shadowColor = C.pink;
        for (const b of bombs) {
          f.g.beginPath();
          f.g.moveTo(b.x, b.y - 12);
          f.g.lineTo(b.x + Math.sin(f.t * 14 + b.x) * 3, b.y);
          f.g.stroke();
        }

        if (over <= 0 || overText !== 'CANNON DOWN') {
          f.g.shadowColor = C.cyan;
          f.g.fillStyle = C.cyan;
          f.g.shadowBlur = 18;
          f.g.beginPath();
          f.g.roundRect(gunX - 22, gunY - 6, 44, 12, 4);
          f.g.fill();
          f.g.beginPath();
          f.g.roundRect(gunX - 4, gunY - 16, 8, 12, 3);
          f.g.fill();
        }
        f.g.restore();

        for (let i = bits.length - 1; i >= 0; i--) {
          const b = bits[i];
          b.life -= f.dt;
          if (b.life <= 0) {
            bits.splice(i, 1);
            continue;
          }
          b.x += b.vx * f.dt;
          b.y += b.vy * f.dt;
          f.g.globalAlpha = Math.min(1, b.life * 1.8);
          f.g.fillStyle = b.colour;
          f.g.fillRect(b.x, b.y, 2.5, 2.5);
        }
        f.g.globalAlpha = 1;

        scoreLine(f, `SCORE ${score}`, `WAVE ${round}`);
        if (over > 0) banner(f, overText, overText === 'WAVE CLEARED' ? C.green : C.pink, Math.min(1, over));
      },
      reset,
    );
  },
};
