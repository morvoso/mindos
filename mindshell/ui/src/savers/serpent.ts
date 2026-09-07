// Serpent: the line that eats and grows. It plays itself — at every step it
// looks at the three ways it could go, throws away the ones that shut it in,
// and takes whichever of the rest gets it closest to the next pellet.

import { backdrop, banner, C, run, scoreLine } from './engine';
import type { Saver } from './types';

interface Cell {
  x: number;
  y: number;
}

const DIRS: Cell[] = [
  { x: 1, y: 0 },
  { x: -1, y: 0 },
  { x: 0, y: 1 },
  { x: 0, y: -1 },
];

export const serpent: Saver = {
  id: 'serpent',
  name: 'Serpent',
  description: 'A line that grows every time it eats. It plays itself.',
  start(canvas) {
    let cell = 20;
    let cols = 10;
    let rows = 10;
    let offX = 0;
    let offY = 0;
    let snake: Cell[] = [];
    let dir: Cell = DIRS[0];
    let food: Cell = { x: 0, y: 0 };
    let score = 0;
    let best = 0;
    let step = 0; // seconds until the next move
    let dead = 0; // seconds the "game over" card still shows

    const at = (list: Cell[], x: number, y: number): boolean => list.some((c) => c.x === x && c.y === y);

    const dropFood = (): void => {
      for (let tries = 0; tries < 400; tries++) {
        const x = Math.floor(Math.random() * cols);
        const y = Math.floor(Math.random() * rows);
        if (!at(snake, x, y)) {
          food = { x, y };
          return;
        }
      }
    };

    const restart = (): void => {
      const y = Math.floor(rows / 2);
      const x = Math.floor(cols / 3);
      snake = [
        { x, y },
        { x: x - 1, y },
        { x: x - 2, y },
      ];
      dir = DIRS[0];
      score = 0;
      step = 0;
      dropFood();
    };

    const reset = (w: number, h: number): void => {
      cell = Math.max(20, Math.min(56, Math.round(Math.min(w, h) / 18)));
      cols = Math.max(8, Math.floor(w / cell));
      rows = Math.max(8, Math.floor(h / cell));
      offX = Math.round((w - cols * cell) / 2);
      offY = Math.round((h - rows * cell) / 2);
      restart();
    };

    /** How much room is left if the head goes to (x, y): a flood fill, so a
     *  move that walls the snake into a pocket can be seen for what it is. */
    const room = (x: number, y: number, body: Cell[]): number => {
      const seen = new Uint8Array(cols * rows);
      for (const c of body) seen[c.y * cols + c.x] = 1;
      const queue: number[] = [y * cols + x];
      seen[y * cols + x] = 1;
      let count = 0;
      while (queue.length) {
        const i = queue.pop() as number;
        count++;
        const cx = i % cols;
        const cy = (i - cx) / cols;
        for (const d of DIRS) {
          const nx = cx + d.x;
          const ny = cy + d.y;
          if (nx < 0 || ny < 0 || nx >= cols || ny >= rows) continue;
          const j = ny * cols + nx;
          if (seen[j]) continue;
          seen[j] = 1;
          queue.push(j);
        }
      }
      return count;
    };

    const think = (): Cell | null => {
      const head = snake[0];
      // The tail moves out of the way as the head moves in, unless we eat.
      const body = snake.slice(0, snake.length - 1);
      let bestMove: Cell | null = null;
      let bestScore = -Infinity;
      for (const d of DIRS) {
        if (d.x === -dir.x && d.y === -dir.y) continue; // never double back
        const nx = head.x + d.x;
        const ny = head.y + d.y;
        if (nx < 0 || ny < 0 || nx >= cols || ny >= rows) continue;
        if (at(body, nx, ny)) continue;
        const free = room(nx, ny, body);
        if (free < snake.length) continue; // that way is a dead end
        const near = Math.abs(nx - food.x) + Math.abs(ny - food.y);
        const value = free * 0.05 - near;
        if (value > bestScore) {
          bestScore = value;
          bestMove = d;
        }
      }
      if (bestMove) return bestMove;
      // Cornered: take whatever still has the most room.
      for (const d of DIRS) {
        if (d.x === -dir.x && d.y === -dir.y) continue;
        const nx = head.x + d.x;
        const ny = head.y + d.y;
        if (nx < 0 || ny < 0 || nx >= cols || ny >= rows) continue;
        if (at(body, nx, ny)) continue;
        const free = room(nx, ny, body);
        if (free > bestScore) {
          bestScore = free;
          bestMove = d;
        }
      }
      return bestMove;
    };

    const move = (): void => {
      const next = think();
      if (!next) {
        best = Math.max(best, score);
        dead = 1.6;
        return;
      }
      dir = next;
      const head = { x: snake[0].x + dir.x, y: snake[0].y + dir.y };
      snake.unshift(head);
      if (head.x === food.x && head.y === food.y) {
        score += 1;
        dropFood();
      } else {
        snake.pop();
      }
    };

    const block = (g: CanvasRenderingContext2D, c: Cell, inset: number): void => {
      const r = Math.max(2, cell * 0.22);
      const x = offX + c.x * cell + inset;
      const y = offY + c.y * cell + inset;
      const s = cell - inset * 2;
      g.beginPath();
      g.roundRect(x, y, s, s, r);
      g.fill();
    };

    return run(
      canvas,
      (f) => {
        backdrop(f);
        // The board: a faint dot in every cell.
        f.g.fillStyle = C.line;
        f.g.globalAlpha = 0.5;
        for (let y = 0; y < rows; y++) {
          for (let x = 0; x < cols; x++) {
            f.g.fillRect(offX + x * cell + cell / 2 - 1, offY + y * cell + cell / 2 - 1, 1.5, 1.5);
          }
        }
        f.g.globalAlpha = 1;

        if (dead > 0) {
          dead -= f.dt;
          if (dead <= 0) restart();
        } else {
          step -= f.dt;
          if (step <= 0) {
            step = Math.max(0.045, 0.11 - score * 0.0012);
            move();
          }
        }

        // The pellet, breathing.
        f.g.save();
        f.g.shadowColor = C.pink;
        f.g.shadowBlur = 18;
        f.g.fillStyle = C.pink;
        f.g.globalAlpha = 0.75 + Math.sin(f.t * 5) * 0.25;
        block(f.g, food, cell * 0.24);
        f.g.restore();

        // The snake: cyan at the head, violet by the tail.
        f.g.save();
        f.g.shadowColor = C.cyan;
        f.g.shadowBlur = 12;
        for (let i = snake.length - 1; i >= 0; i--) {
          const k = snake.length < 2 ? 0 : i / (snake.length - 1);
          f.g.fillStyle = i === 0 ? '#c9f7ff' : k < 0.5 ? C.cyan : C.violet;
          f.g.globalAlpha = dead > 0 ? 0.25 + 0.4 * Math.abs(Math.sin(dead * 12)) : 1 - k * 0.35;
          block(f.g, snake[i], cell * 0.12);
        }
        f.g.restore();

        scoreLine(f, `LENGTH ${snake.length}`, best ? `BEST ${best + 3}` : '');
        if (dead > 0) banner(f, 'CAUGHT', C.pink, Math.min(1, dead));
      },
      reset,
    );
  },
};
