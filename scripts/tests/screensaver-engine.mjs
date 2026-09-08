#!/usr/bin/env node
// Exercise the real animation loop with a deterministic browser clock.
import assert from 'node:assert/strict';
import { run } from '../../mindshell/ui/src/savers/engine.ts';
let now = 0;
let nextId = 0;
const callbacks = new Map();
const timers = new Map();
const motion = Object.assign(new EventTarget(), { matches: false });
globalThis.window = { devicePixelRatio: 2, matchMedia: () => motion };
globalThis.document = Object.assign(new EventTarget(), { hidden: false });
globalThis.performance = { now: () => now };
globalThis.requestAnimationFrame = fn => { callbacks.set(++nextId, fn); return nextId; };
globalThis.cancelAnimationFrame = id => callbacks.delete(id);
globalThis.setTimeout = (fn, delay) => { timers.set(++nextId, { fn, due: now + delay }); return nextId; };
globalThis.clearTimeout = id => timers.delete(id);
globalThis.ResizeObserver = class { observe() {} disconnect() {} };
let viewport = { width: 3840, height: 2160 };
const canvas = {
  width: 0, height: 0,
  getContext: () => ({ setTransform() {}, fillRect() {} }),
  getBoundingClientRect: () => viewport,
};
function tick(time) {
  now = time;
  for (const [id, timer] of timers) {
    if (timer.due <= now) { timers.delete(id); timer.fn(); }
  }
  const pending = [...callbacks.values()];
  callbacks.clear();
  for (const fn of pending) fn(now);
}
for (const hz of [60, 75, 120, 144, 240]) {
  now = 0;
  let frames = 0;
  let elapsed = 0;
  const stop = run(canvas, f => { frames++; elapsed += f.dt; });
  for (let i = 1; i <= hz; i++) tick(i * 1000 / hz);
  assert.ok(frames >= 23 && frames <= 25, `${hz} Hz: ${frames} frames`);
  assert.ok(elapsed > 0.95 && elapsed <= 1.01, `simulation time: ${elapsed}`);
  assert.ok(canvas.width * canvas.height <= 1920 * 1080);
  document.hidden = true;
  document.dispatchEvent(new Event('visibilitychange'));
  assert.equal(callbacks.size, 0);
  assert.equal(timers.size, 0);
  now += 5000;
  document.hidden = false;
  document.dispatchEvent(new Event('visibilitychange'));
  const before = elapsed;
  tick(now + 16);
  assert.ok(elapsed - before < 0.05, 'resume must not catch up hidden time');
  motion.matches = true;
  motion.dispatchEvent(new Event('change'));
  tick(now + 16);
  assert.equal(callbacks.size, 0, 'reduced motion paints one still frame');
  assert.equal(timers.size, 0);
  motion.matches = false;
  motion.dispatchEvent(new Event('change'));
  assert.equal(callbacks.size, 1);
  stop();
  assert.equal(callbacks.size, 0);
  assert.equal(timers.size, 0);
  motion.dispatchEvent(new Event('change'));
  assert.equal(callbacks.size, 0);
}
for (const [width, height] of [[1920, 1440], [3440, 1440], [2560, 1600], [5120, 1440]]) {
  viewport = { width, height };
  const stop = run(canvas, () => {});
  assert.ok(canvas.width * canvas.height <= 1920 * 1080, `pixel budget at ${width}×${height}`);
  stop();
}
console.log('Screensaver: frame/pixel budgets, pause/resume, reduced motion and cleanup passed.');
