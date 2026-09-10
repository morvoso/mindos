#!/usr/bin/env node
// Built-in Node WebSocket + Chromium DevTools: no browser test dependency.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { mkdtemp, mkdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { resolve } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { fileURLToPath, pathToFileURL } from 'node:url';

const root = fileURLToPath(new URL('../../', import.meta.url));
const profile = await mkdtemp(resolve(tmpdir(), 'mindos-ui-test-'));
const out = resolve(root, 'build/shots/gaming-smoke');
await mkdir(out, { recursive: true });
const chrome = spawn(process.env.CHROMIUM || 'chromium', [
  '--headless', '--disable-gpu', '--no-sandbox', '--no-first-run',
  '--remote-debugging-port=0', `--user-data-dir=${profile}`, 'about:blank',
], { stdio: ['ignore', 'ignore', 'pipe'] });
let socket;
try {
  const address = await new Promise((done, reject) => {
    let log = '';
    const timeout = setTimeout(() => reject(new Error('Chromium did not start')), 15000);
    chrome.once('error', reject);
    chrome.stderr.on('data', (data) => {
      log = (log + data).slice(-8192);
      const match = log.match(/DevTools listening on ws:\/\/([^/]+)/);
      if (match) { clearTimeout(timeout); done(`http://${match[1]}`); }
    });
  });
  const target = await (await fetch(`${address}/json/new?about:blank`, { method: 'PUT' })).json();
  socket = new WebSocket(target.webSocketDebuggerUrl);
  await once(socket, 'open');
  let next = 0;
  const requests = new Map();
  const errors = [];
  socket.addEventListener('message', ({ data }) => {
    const message = JSON.parse(data);
    if (message.method === 'Runtime.exceptionThrown') errors.push(message.params.exceptionDetails);
    if (!message.id) return;
    const request = requests.get(message.id);
    if (!request) return;
    requests.delete(message.id);
    clearTimeout(request.timeout);
    if (message.error) request.reject(new Error(JSON.stringify(message.error)));
    else request.done(message.result);
  });
  const send = (method, params = {}) => new Promise((done, reject) => {
    const id = ++next;
    const timeout = setTimeout(() => { requests.delete(id); reject(new Error(`${method} timed out`)); }, 10000);
    requests.set(id, { done, reject, timeout });
    socket.send(JSON.stringify({ id, method, params }));
  });
  const evaluate = async (expression) => {
    const result = await send('Runtime.evaluate', { expression: `{${expression}\n}`, returnByValue: true, awaitPromise: true });
    if (result.exceptionDetails) throw new Error(JSON.stringify(result.exceptionDetails));
    return result.result.value;
  };
  const waitFor = async (expression) => {
    for (let i = 0; i < 100; i++) {
      if (await evaluate(expression)) return;
      await delay(30);
    }
    throw new Error(`Not ready: ${expression}`);
  };
  const clickNav = async (text) => {
    await evaluate(`[...document.querySelectorAll('.nav-item')].find(b => b.textContent === ${JSON.stringify(text)}).click()`);
    await delay(80);
  };
  const shot = async (name) => {
    await evaluate('document.fonts.ready');
    await delay(160);
    const { data } = await send('Page.captureScreenshot', { format: 'png' });
    await writeFile(resolve(out, name + '.png'), Buffer.from(data, 'base64'));
  };
  await send('Runtime.enable');
  const page = pathToFileURL(resolve(root, 'mindshell/ui/dist/index.html')).href;
  const open = async (query, width = 1440, height = 1000) => {
    await send('Emulation.setDeviceMetricsOverride', { width, height, deviceScaleFactor: 1, mobile: false });
    await send('Page.navigate', { url: page + '?' + query });
    await waitFor("document.querySelector('.game-library')?.getAttribute('aria-busy') === 'false'");
  };
  await open('kind=desktop&output=Virtual-1');
  assert.equal(await evaluate("document.querySelector('.game-feature h1').textContent"), 'Cyberpunk 2077');
  assert.equal(await evaluate("document.querySelectorAll('.game-tile').length"), 8);
  await shot('desktop-dark');
  assert.equal(await evaluate("document.querySelector('.appearance-controls button').textContent"), '');
  assert.equal(await evaluate("document.querySelector('.gaming-top-status')"), null);
  assert.doesNotMatch(await evaluate("document.querySelector('.gaming-nav').textContent"), /Capture|Recording|Screenshot/);
  await evaluate("document.querySelector('.gaming-nav [data-page=settings]').click()");
  await waitFor("!!document.querySelector('.embedded-app .page-home')");
  assert.match(await evaluate("document.querySelector('.gaming-edition').textContent"), /Settings/);
  await shot('desktop-settings');
  await evaluate("document.querySelector('.desktop-panel-toolbar button').click(); document.querySelectorAll('.mode-switch button')[1].click()");
  await waitFor("!!document.querySelector('.productivity-workspace')");
  assert.doesNotMatch(await evaluate("document.querySelector('.gaming-nav').textContent"), /Gaming Center|Capture/);
  await evaluate("const n = document.querySelector('.work-notes'); n.value = 'Finish the budget'; n.dispatchEvent(new Event('input'))");
  await shot('desktop-productivity');
  await evaluate("document.querySelectorAll('.mode-switch button')[0].click()");
  await waitFor("document.querySelector('.game-library')?.getAttribute('aria-busy') === 'false'");
  await evaluate("document.querySelectorAll('.mode-switch button')[1].click()");
  assert.equal(await evaluate("document.querySelector('.work-notes').value"), 'Finish the budget');
  await evaluate("window.mindos._dispatch('outputs', { outputs: [{name:'Virtual-1',width:1440,height:1000,x:0,y:0,scale:1,primary:false},{name:'Virtual-2',width:1440,height:1000,x:1440,y:0,scale:1,primary:true}] })");
  assert.equal(await evaluate("document.querySelector('.gaming-workspace')"), null, 'Secondary output has no workspace');
  assert.equal(await evaluate("getComputedStyle(document.querySelector('.desktop-icons')).display"), 'none');
  await shot('secondary-wallpaper');
  await evaluate("window.mindos._dispatch('outputs', { outputs: [{name:'Virtual-1',width:1440,height:1000,x:0,y:0,scale:1,primary:true},{name:'Virtual-2',width:1440,height:1000,x:1440,y:0,scale:1}] })");
  assert.equal(await evaluate("document.querySelector('.work-notes').value"), 'Finish the budget');
  await evaluate("document.querySelectorAll('.mode-switch button')[0].click()");
  await waitFor("document.querySelector('.game-library')?.getAttribute('aria-busy') === 'false'");

  await evaluate("document.querySelector('[aria-label=\"Use light theme\"]').click()");
  assert.equal(await evaluate("document.documentElement.dataset.theme"), 'light');
  await shot('desktop-light');
  await open('kind=desktop&output=Virtual-1');
  assert.equal(await evaluate("document.documentElement.dataset.theme"), 'light');
  await evaluate("document.querySelector('[aria-label=\"Use dark theme\"]').click()");
  assert.equal(await evaluate("document.querySelector('.live-background')"), null, 'No wallpaper animation canvas');
  assert.equal(await evaluate("document.querySelector('[aria-label=\"Live background\"]')"), null, 'Animated wallpaper control removed');
  assert.match(await evaluate("getComputedStyle(document.querySelector('.game-library')).backdropFilter"), /blur/, 'Desktop window has frosted glass');
  await evaluate("window.mindos.call('shell.state').then(state => { window.savedOutputs = state.outputs; window.mindos._dispatch('outputs', {outputs:state.outputs.map(o => ({...o,software_rendering:true}))}); })");
  assert.equal(await evaluate("getComputedStyle(document.querySelector('.game-library')).backdropFilter"), 'none', 'CPU renderer does not run a live blur filter');
  assert.match(await evaluate("getComputedStyle(document.querySelector('.game-library')).backgroundImage"), /data:image\/png/, 'CPU renderer uses a cached frosted bitmap');
  await evaluate("window.mindos._dispatch('outputs', {outputs:window.savedOutputs})");
  await evaluate("window.mindos._dispatch('game', {running: true})");
  assert.match(await evaluate("document.querySelector('.gaming-metrics').textContent"), /Sampling paused/);
  await evaluate("window.mindos._dispatch('game', {running: false})");

  await evaluate(`window.libraryCalls = []; window.originalCall = window.mindos.call;
    window.mindos.call = function(method, params) {
      window.libraryCalls.push({method, params}); return window.originalCall.call(this, method, params);
    }; document.querySelector('.game-play').click()`);
  await waitFor("!document.querySelector('.game-play').disabled && document.querySelector('.game-play').textContent.includes('Return')");
  assert.equal(await evaluate("window.libraryCalls.filter(c => c.method === 'shell.run' && c.params.argv[1] === 'launch').length"), 1);
  await evaluate("document.querySelector('.game-play').click()");
  await waitFor("!document.querySelector('.game-play').disabled");
  assert.equal(await evaluate("window.libraryCalls.filter(c => c.method === 'windows.focus').length"), 1);
  await evaluate("document.querySelector('.game-favorite').click(); document.querySelectorAll('.game-tab')[1].click()");
  assert.equal(await evaluate("document.querySelectorAll('.game-tile').length"), 1);
  await open('kind=app&id=library', 1040, 700);
  assert.equal(await evaluate("document.querySelector('.game-favorite').getAttribute('aria-pressed')"), 'true');
  await evaluate("const s=document.querySelector('.game-search'); s.value='hollow';s.dispatchEvent(new Event('input'))");
  assert.equal(await evaluate("document.querySelectorAll('.game-tile').length"), 1);
  assert.equal(await evaluate("document.querySelector('.game-tile strong').textContent"), 'Hollow Knight');
  await evaluate("document.querySelector('.game-tile').click()");
  assert.equal(await evaluate("document.querySelector('.game-feature h1').textContent"), 'Hollow Knight');
  await shot('library-window');
  await evaluate(`window.mindos.call = async (method, params) => {throw new Error('Launcher unavailable')}; document.querySelector('.game-play').click()`);
  await waitFor("document.querySelector('.gaming-message').textContent.includes('Launcher unavailable') && !document.querySelector('.game-play').disabled");
  await open('kind=desktop&output=Virtual-1&library=empty');
  assert.equal(await evaluate("document.querySelector('.game-empty h3').textContent"), 'No installed games');
  await shot('empty-library');
  await open('kind=desktop&output=Virtual-1&library=missing');
  assert.equal(await evaluate("document.querySelector('.gaming-message').dataset.error"), 'true');
  await open('kind=desktop&output=Virtual-1', 800, 700);
  assert.equal(await evaluate("const el=document.querySelector('.game-library-body');el.scrollWidth <= el.clientWidth"), true);
  await shot('desktop-compact');
  await evaluate("document.querySelector('[aria-label=\"Close library\"]').click()");
  assert.equal(await evaluate("document.querySelector('.gaming-main').hidden"), true);
  await evaluate("document.querySelector('.gaming-nav-link').click()");
  assert.equal(await evaluate("document.querySelector('.gaming-main').hidden"), false);
  await open('kind=preview', 1920, 1080);
  await shot('desktop-with-shelf');
  await evaluate("document.querySelector('[aria-label=\"Use light theme\"]').click()");
  await shot('desktop-light-with-shelf');
  await evaluate("document.querySelector('[aria-label=\"Use dark theme\"]').click(); document.querySelector('[aria-label=\"Close library\"]').click()");
  await shot('desktop-live-background');
  await send('Page.navigate', { url: page + '?kind=greeter&arg=%7B%22primary%22%3Atrue%7D' });
  await waitFor("document.querySelector('.g-pw') !== null");
  await delay(800);
  await shot('login-dark');
  await evaluate("document.querySelector('[aria-label=\"Use light theme\"]').click()");
  await shot('login-light');

  // Newly wired gaming tools: both success and missing-connection states.
  const openTool = async (name, arg = {}, width = 1200) => {
    await send('Emulation.setDeviceMetricsOverride', { width, height: 900, deviceScaleFactor: 1, mobile: false });
    await send('Page.navigate', { url: page + `?kind=app&id=${name}&arg=${encodeURIComponent(JSON.stringify(arg))}` });
    await waitFor("document.querySelector('.gaming-app') !== null");
    await waitFor("!document.querySelector('.play-content') || !document.querySelector('.play-content').textContent.includes('Loading…')");
  };
  const toolButton = async label => {
    await evaluate(`[...document.querySelectorAll('button')].find(b => b.textContent === ${JSON.stringify(label)}).click()`);
    await delay(150);
    await waitFor("document.body.getAttribute('aria-busy') !== 'true'");
  };
  await openTool('gaming');
  assert.ok(await evaluate("document.querySelector('.gaming-app').getBoundingClientRect().width > 1000"), 'Gaming app fills its window');
  assert.match(await evaluate('document.body.textContent'), /Downloads/);
  await shot('gaming-downloads');
  // Storage moves need the cold drive first; the tab says so instead of failing.
  await toolButton('storage');
  await toolButton('Plan move to cold drive');
  assert.match(await evaluate('document.body.textContent'), /cold storage folder in Connections/);
  await toolButton('connections');
  // Fields are found by their label: the pages grow, the indices move.
  const setField = (starts, value) => evaluate(`[...document.querySelectorAll('.play-field')].find(f => f.textContent.startsWith(${JSON.stringify(starts)})).querySelector('input').value = ${JSON.stringify(value)}`);
  await setField('Synced save folder', '/cloud');
  await setField('Cold storage', '/cold');
  await toolButton('Save locations');
  assert.match(await evaluate("document.querySelectorAll('.play-field input')[1].value"), /\/cold/);
  await toolButton('saves');
  await setField('Save folder', '/saves');
  await toolButton('Set save folder');
  await toolButton('Back up locally');
  assert.match(await evaluate('document.body.textContent'), /6 files/);
  assert.doesNotMatch(await evaluate('document.body.textContent'), /Invalid Date/);
  await toolButton('storage');
  await toolButton('Plan move to cold drive');
  assert.match(await evaluate('document.body.textContent'), /42.0 GiB/);
  await toolButton('Move game');
  assert.match(await evaluate('document.body.textContent'), /Plan restore/);
  await toolButton('downloads');
  assert.equal(await evaluate("document.querySelector('progress').value > 0"), true);
  await toolButton('audio');
  assert.match(await evaluate('document.body.textContent'), /Per-app audio/);
  await toolButton('history');
  assert.equal(await evaluate("document.querySelectorAll('.play-card polyline').length"), 1);
  await shot('gaming-history');
  await toolButton('activity');
  assert.match(await evaluate('document.body.textContent'), /While you were away/);
  await send('Emulation.setDeviceMetricsOverride', { width: 1200, height: 900, deviceScaleFactor: 1, mobile: false });
  await send('Page.navigate', { url: page + '?kind=greeter' });
  await waitFor("document.querySelector('.g-pw') !== null");
  await toolButton('On-screen keyboard · controller A');
  assert.equal(await evaluate("document.querySelectorAll('.play-osk').length"), 1);
  await evaluate("document.querySelector('.play-osk button').click()");
  assert.equal(await evaluate("document.querySelector('.g-pw').value"), '1');
  assert.equal(await evaluate("document.querySelector('.g-pw').type"), 'password');
  await toolButton('Done');
  assert.equal(await evaluate("document.querySelectorAll('.play-osk').length"), 0);
  assert.deepEqual(errors, [], 'No uncaught browser exceptions');
  console.log('PASS: discovery preview, launch/focus/error recovery, search, favorites, empty/missing helpers, dark/light persistence, static wallpaper, frosted glass, GameMode pause, compact layout, login keyboard, gaming downloads/saves/storage/connections/audio/history/activity');
  console.log(`Screenshots: ${out}`);
} finally {
  socket?.close();
  chrome.kill('SIGTERM');
  if (chrome.exitCode === null) await once(chrome, 'exit');
  await rm(profile, { recursive: true, force: true, maxRetries: 5, retryDelay: 100 });
}
