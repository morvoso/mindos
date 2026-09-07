// Volley: two bats and a ball. Both sides are played by the machine, each
// with its own reaction time and its own habit of aiming a little off, so
// the rallies go somewhere and the score keeps moving.

import { backdrop, banner, C, clamp, rand, run, scoreLine } from './engine';
import type { Saver } from './types';

interface Bat {
  y: number;
  aim: number;
  /** Seconds until this side looks at the ball again. */
  think: number;
  /** How far off it aims, in bat heights. */
  sloppy: number;
  score: number;
  glow: number;
}

export const volley: Saver = {
  id: 'volley',
  name: 'Volley',
  description: 'Two bats, one ball, nobody at the controls.',
  start(canvas) {
    let w = 0;
    let h = 0;
    let batW = 12;
    let batH = 120;
    let inset = 60;
    let radius = 8;
    let bx = 0;
    let by = 0;
    let vx = 0;
    let vy = 0;
    let speed = 460;
    let serve = 1;
    let pause = 0;
    let over = 0;
    const trail: { x: number; y: number }[] = [];
    const left: Bat = { y: 0, aim: 0, think: 0, sloppy: 0.3, score: 0, glow: 0 };
    const right: Bat = { y: 0, aim: 0, think: 0, sloppy: 0.3, score: 0, glow: 0 };

    const serveBall = (): void => {
      bx = w / 2;
      by = h / 2;
      speed = Math.max(340, Math.min(h, 900) * 0.55);
      const angle = rand(-0.42, 0.42);
      vx = Math.cos(angle) * speed * serve;
      vy = Math.sin(angle) * speed;
      pause = 0.7;
      trail.length = 0;
    };

    const reset = (nw: number, nh: number): void => {
      w = nw;
      h = nh;
      batH = clamp(h * 0.17, 54, 190);
      batW = clamp(h * 0.014, 8, 18);
      inset = clamp(w * 0.055, 28, 90);
      radius = clamp(h * 0.011, 5, 13);
      left.y = right.y = h / 2;
      left.aim = right.aim = h / 2;
      left.score = right.score = 0;
      serve = Math.random() < 0.5 ? -1 : 1;
      serveBall();
    };

    /** Where the ball will cross `x`, bounces off the top and bottom included. */
    const predict = (x: number): number => {
      if (Math.abs(vx) < 1) return h / 2;
      const time = (x - bx) / vx;
      if (time <= 0) return h / 2;
      const span = h - radius * 2;
      let y = by - radius + vy * time;
      y = ((y % (span * 2)) + span * 2) % (span * 2);
      return radius + (y > span ? span * 2 - y : y);
    };

    const drive = (bat: Bat, x: number, dt: number): void => {
      bat.think -= dt;
      if (bat.think <= 0) {
        bat.think = rand(0.06, 0.2);
        // Aim for where the ball is heading, but not exactly; the miss gets
        // more likely the faster the ball is going.
        const wobble = (Math.random() - 0.5) * batH * bat.sloppy * (speed / 500);
        bat.aim = predict(x) + wobble;
        if (Math.random() < 0.04) bat.aim += (Math.random() < 0.5 ? -1 : 1) * batH * 0.9;
      }
      const target = clamp(bat.aim, batH / 2, h - batH / 2);
      const top = Math.min(h * 1.9, 720);
      const move = clamp(target - bat.y, -top * dt, top * dt);
      bat.y = clamp(bat.y + move, batH / 2, h - batH / 2);
      bat.glow = Math.max(0, bat.glow - dt * 3);
    };

    const bounceOff = (bat: Bat, side: number): void => {
      const offset = clamp((by - bat.y) / (batH / 2), -1, 1);
      speed = Math.min(speed * 1.06, Math.max(900, h * 1.4));
      const angle = offset * 0.9;
      vx = Math.cos(angle) * speed * side;
      vy = Math.sin(angle) * speed;
      bat.glow = 1;
    };

    const point = (bat: Bat, direction: number): void => {
      bat.score += 1;
      serve = direction;
      if (bat.score >= 9) {
        over = 2.4;
      } else {
        serveBall();
      }
    };

    return run(
      canvas,
      (f) => {
        backdrop(f, 0.34);

        // The court.
        f.g.strokeStyle = C.line;
        f.g.lineWidth = 2;
        f.g.setLineDash([10, 14]);
        f.g.beginPath();
        f.g.moveTo(f.w / 2, 10);
        f.g.lineTo(f.w / 2, f.h - 10);
        f.g.stroke();
        f.g.setLineDash([]);

        if (over > 0) {
          over -= f.dt;
          if (over <= 0) {
            left.score = 0;
            right.score = 0;
            left.sloppy = rand(0.2, 0.5);
            right.sloppy = rand(0.2, 0.5);
            serveBall();
          }
        } else if (pause > 0) {
          pause -= f.dt;
        } else {
          bx += vx * f.dt;
          by += vy * f.dt;
          if (by < radius && vy < 0) {
            by = radius;
            vy = -vy;
          }
          if (by > f.h - radius && vy > 0) {
            by = f.h - radius;
            vy = -vy;
          }
          const lx = inset + batW;
          const rx = f.w - inset - batW;
          if (vx < 0 && bx - radius <= lx && bx - radius > lx - 40 && Math.abs(by - left.y) < batH / 2 + radius) {
            bx = lx + radius;
            bounceOff(left, 1);
          }
          if (vx > 0 && bx + radius >= rx && bx + radius < rx + 40 && Math.abs(by - right.y) < batH / 2 + radius) {
            bx = rx - radius;
            bounceOff(right, -1);
          }
          if (bx < -40) point(right, 1);
          else if (bx > f.w + 40) point(left, -1);

          trail.push({ x: bx, y: by });
          if (trail.length > 14) trail.shift();
        }

        drive(left, inset + batW, f.dt);
        drive(right, f.w - inset - batW, f.dt);

        // The bats.
        f.g.save();
        for (const [bat, x, colour] of [
          [left, inset, C.cyan],
          [right, f.w - inset - batW, C.violet],
        ] as [Bat, number, string][]) {
          f.g.shadowColor = colour;
          f.g.shadowBlur = 14 + bat.glow * 26;
          f.g.fillStyle = colour;
          f.g.beginPath();
          f.g.roundRect(x, bat.y - batH / 2, batW, batH, batW / 2);
          f.g.fill();
        }
        f.g.restore();

        // The ball and what it left behind.
        f.g.save();
        f.g.shadowColor = C.cyan;
        f.g.shadowBlur = 20;
        for (let i = 0; i < trail.length; i++) {
          f.g.globalAlpha = (i / trail.length) * 0.35;
          f.g.fillStyle = C.cyan;
          f.g.beginPath();
          f.g.arc(trail[i].x, trail[i].y, radius * (0.3 + (i / trail.length) * 0.7), 0, Math.PI * 2);
          f.g.fill();
        }
        f.g.globalAlpha = 1;
        f.g.fillStyle = '#dffaff';
        f.g.beginPath();
        f.g.arc(bx, by, radius, 0, Math.PI * 2);
        f.g.fill();
        f.g.restore();

        scoreLine(f, String(left.score), String(right.score));
        if (over > 0) banner(f, left.score > right.score ? 'LEFT WINS' : 'RIGHT WINS', left.score > right.score ? C.cyan : C.violet, Math.min(1, over));
      },
      reset,
    );
  },
};
