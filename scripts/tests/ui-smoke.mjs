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
const out = resolve(root, 'build/shots/ui-smoke');
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
    const { data } = await send('Page.captureScreenshot', { format: 'png' });
    await writeFile(resolve(out, name + '.png'), Buffer.from(data, 'base64'));
  };
  await send('Runtime.enable');
  await send('Emulation.setDeviceMetricsOverride', { width: 1040, height: 760, deviceScaleFactor: 1, mobile: false });
  await send('Page.navigate', { url: pathToFileURL(resolve(root, 'mindshell/ui/dist/index.html')).href + '?kind=app&id=settings' });
  await waitFor("document.querySelector('.page-home .pill')?.textContent === 'Balanced'");
  await shot('overview');
  assert.equal(await evaluate("document.querySelector('[aria-current=page]').textContent"), 'Overview');

  // The overview opens the configured native terminal and reports launch errors.
  await evaluate(`window.terminalCall = window.mindos.call; window.terminalLaunches = [];
    window.mindos.call = async function(method, params) {
      if (method === 'shell.exec') { window.terminalLaunches.push(params.cmd); return null; }
      return window.terminalCall.call(this, method, params);
    }; document.querySelectorAll('.hero-actions button')[1].click()`);
  await waitFor("!document.querySelectorAll('.hero-actions button')[1].disabled");
  assert.deepEqual(await evaluate('window.terminalLaunches'), ['kitty']);
  await evaluate(`window.mindos.call = async function(method, params) {
    if (method === 'shell.exec') throw new Error('Terminal unavailable');
    return window.terminalCall.call(this, method, params);
  }; document.querySelectorAll('.hero-actions button')[1].click()`);
  await waitFor("document.querySelector('.notice')?.dataset.kind === 'error' && !document.querySelectorAll('.hero-actions button')[1].disabled");
  assert.equal(await evaluate("document.querySelector('.notice').textContent.includes('Terminal unavailable')"), true);
  await evaluate('window.mindos.call = window.terminalCall');

  await evaluate("document.querySelector('.app-settings').dispatchEvent(new KeyboardEvent('keydown', {key: 'k', ctrlKey: true, bubbles: true}))");
  assert.equal(await evaluate("document.activeElement.classList.contains('nav-search-input')"), true);
  await evaluate("document.activeElement.value = 'gpu'; document.activeElement.dispatchEvent(new Event('input'))");
  assert.deepEqual(await evaluate("[...document.querySelectorAll('.nav-item')].filter(b => !b.hidden).map(b => b.textContent)"), ['Performance']);
  await evaluate("document.activeElement.dispatchEvent(new KeyboardEvent('keydown', {key: 'Enter', bubbles: true}))");
  await waitFor("document.querySelector('.perf-card.on') !== null");
  await evaluate("const s = document.querySelector('.nav-search-input'); s.value = ''; s.dispatchEvent(new Event('input'))");
  await shot('performance');
  assert.equal(await evaluate("document.querySelector('.settings-advanced').open"), false);
  await evaluate("document.querySelector('.perf-performance').click()");
  await waitFor("document.querySelector('.perf-performance').getAttribute('aria-pressed') === 'true' && !document.querySelector('.perf-performance').disabled");
  await evaluate("document.querySelector('.settings-advanced').open = true");
  assert.equal(await evaluate("document.querySelector('.settings-advanced select').value"), 'scx_lavd');
  await evaluate("const selects = document.querySelectorAll('.settings-advanced select'); selects[1].value = 'max'; selects[1].dispatchEvent(new Event('change'))");
  await waitFor("!document.querySelector('.settings-advanced select').disabled");
  assert.equal(await evaluate("document.querySelectorAll('.settings-advanced select')[1].value"), 'max');
  const runningBefore = await evaluate("document.querySelector('.kv').textContent");
  await evaluate("const s = document.querySelector('.settings-advanced select'); s.value = ''; s.dispatchEvent(new Event('change'))");
  await waitFor("!document.querySelector('.settings-advanced select').disabled");
  assert.equal(await evaluate("document.querySelector('.settings-advanced select').value"), '');
  assert.equal(await evaluate("document.querySelector('.kv').textContent"), runningBefore, 'A saved scheduler must not impersonate the running scheduler');
  assert.equal(await evaluate("document.querySelectorAll('.row input:not([aria-labelledby]), .row select:not([aria-labelledby])').length"), 0);

  // Errors must restore the confirmed selection and leave the controls usable.
  await evaluate(`window.originalCall = window.mindos.call; window.mindos.call = async function(method, params) {
    if (method === 'shell.run' && params.argv.includes('set')) throw new Error('Simulated write failure');
    return window.originalCall.call(this, method, params);
  }; document.querySelector('.perf-quiet').click()`);
  await waitFor("document.querySelector('.notice')?.dataset.kind === 'error' && !document.querySelector('.perf-performance').disabled");
  assert.equal(await evaluate("document.querySelector('.perf-card.on').classList.contains('perf-performance')"), true);
  await evaluate('window.mindos.call = window.originalCall');

  // Minimal gaming setup: cancellation/failure remain retryable, and changing
  // pages while installing follows the same transaction without duplicating it.
  await evaluate(`window.gamingMissing = true; window.gamingInstallCalls = 0; window.gamingOutcome = 'cancel';
    window.mindos.call = async function(method, params) {
      if (method === 'shell.run' && params.argv[0] === 'mindos-dlss' && window.gamingMissing)
        throw new Error('mindos-dlss: No such file or directory (os error 2)');
      if (method === 'shell.run' && params.argv[0] === 'pkexec' && params.argv.includes('mindos-gaming')) {
        window.gamingInstallCalls++;
        window.gamingInstallArgs = params.argv;
        if (window.gamingOutcome === 'cancel') return {ok: false, status: 126, stdout: '', stderr: ''};
        if (window.gamingOutcome === 'failure') return {ok: false, status: 1, stdout: 'Download unavailable', stderr: ''};
        await new Promise(resolve => { window.finishGamingInstall = resolve; });
        window.gamingMissing = false;
        return {ok: true, status: 0, stdout: 'Installed', stderr: ''};
      }
      return window.originalCall.call(this, method, params);
    }`);
  await clickNav('Games');
  await waitFor("document.querySelector('.games-setup')?.hidden === false && document.querySelector('.page').getAttribute('aria-busy') === 'false'");
  assert.equal(await evaluate("document.querySelector('.dlss-games').closest('.card').hidden"), true);
  await shot('games-setup');
  await evaluate("[...document.querySelectorAll('button')].find(b => b.textContent === 'Install gaming tools').click()");
  await waitFor("document.querySelector('.notice')?.textContent.includes('Installation cancelled')");
  assert.equal(await evaluate("document.querySelector('.games-setup button').disabled"), false);
  await evaluate("window.gamingOutcome = 'failure'; document.querySelector('.games-setup button').click()");
  await waitFor("document.querySelector('.notice')?.textContent.includes('Download unavailable')");
  await evaluate("window.gamingOutcome = 'success'; const b = document.querySelector('.games-setup button'); b.click(); b.click()");
  await waitFor("typeof window.finishGamingInstall === 'function' && document.querySelector('.games-setup button').disabled");
  assert.equal(await evaluate('window.gamingInstallCalls'), 3);
  await clickNav('Software');
  await clickNav('Games');
  await waitFor("document.querySelector('.games-setup button')?.textContent === 'Installing…'");
  assert.equal(await evaluate('window.gamingInstallCalls'), 3);
  assert.equal(await evaluate("document.querySelector('.games-setup button').disabled"), true);
  await evaluate('new Promise(resolve => setTimeout(resolve, 6500))');
  assert.equal(await evaluate("document.querySelector('.notice').hidden"), false, 'Installation status stays visible during a long download');
  await evaluate('window.finishGamingInstall()');
  await waitFor("document.querySelector('.games-setup')?.hidden === true && document.querySelector('.notice')?.dataset.kind === 'ok'");
  assert.deepEqual(await evaluate('window.gamingInstallArgs'), ['pkexec', 'mindos-pkg', 'install', '--repo-only', 'mindos-gaming']);
  assert.equal(await evaluate("document.querySelectorAll('.dlss-game').length"), 3);
  await evaluate('window.mindos.call = window.originalCall');
  await clickNav('Overview');

  // The gaming desktop exposes Software instead of development setup.
  assert.equal(await evaluate("[...document.querySelectorAll('.nav-item')].some(b => b.textContent === 'Developer')"), false);
  await clickNav('Software');
  await evaluate(`window.softwareCalls = []; window.mindos.call = async function(method, params) {
    if (method === 'shell.exec') { window.softwareCalls.push(params.cmd); return null; }
    return window.originalCall.call(this, method, params);
  }`);
  await evaluate("[...document.querySelectorAll('button')].find(b => b.textContent.includes('Open Software')).click()");
  await evaluate("[...document.querySelectorAll('button')].find(b => b.textContent.includes('Choose installer')).click()");
  assert.deepEqual(await evaluate('window.softwareCalls'), ['gio launch /usr/share/applications/octopi.desktop', 'mindos-win-open']);
  await evaluate('window.mindos.call = window.originalCall');

  // Games: failed scans must not masquerade as an empty library. Retry uses
  // the same filtered scan; duplicate keyboard/click activation is suppressed.
  await evaluate(`window.failGameScan = true; window.swapCalls = 0;
    window.mindos.call = async function(method, params) {
      if (method === 'shell.run' && params.argv[0] === 'mindos-dlss') {
        if (params.argv[2] === 'games') throw new Error('Unexpected unfiltered rescan');
        if (params.argv[2] === 'scan' && window.failGameScan)
          return {ok: false, status: 1, stdout: '', stderr: '', json: {error: 'Simulated game scan failure'}};
        if (params.argv[2] === 'library' && window.corruptGameLibrary)
          return {ok: true, status: 0, stdout: 'library is empty\\n[]', stderr: '', json: null};
        if (params.argv[2] === 'swap') {
          window.swapCalls++;
          await new Promise(resolve => { window.finishSwap = resolve; });
        }
      }
      return window.originalCall.call(this, method, params);
    }`);
  await clickNav('Games');
  await waitFor("document.querySelector('.notice')?.textContent.includes('Simulated game scan failure')");
  assert.equal(await evaluate("document.querySelector('.dlss-games').textContent.includes('No games with an upscaler found')"), false);
  assert.equal(await evaluate("[...document.querySelectorAll('button')].find(b => b.textContent === 'Get a version…').disabled"), true);
  await evaluate("window.failGameScan = false; window.corruptGameLibrary = true; document.querySelector('[aria-label=\"Scan installed games again\"]').click()");
  await waitFor("document.querySelector('.notice')?.textContent.includes('unreadable response') && document.querySelector('.page').getAttribute('aria-busy') === 'false'");
  await evaluate("window.corruptGameLibrary = false; document.querySelector('[aria-label=\"Scan installed games again\"]').click()");
  await waitFor("document.querySelectorAll('.dlss-game').length === 3 && document.querySelector('.page').getAttribute('aria-busy') === 'false'");
  assert.equal(await evaluate("document.querySelector('.notice').hidden"), true);
  await evaluate("const s = document.querySelector('.dlss-search'); s.value = 'wukong'; s.dispatchEvent(new Event('input'))");
  assert.equal(await evaluate("document.querySelectorAll('.dlss-game').length"), 1);
  await shot('games-filtered');
  await evaluate("const s = document.querySelector('.dlss-search'); s.value = ''; s.dispatchEvent(new Event('input')); const b = document.querySelector('.dlss-dll button'); b.click(); b.click()");
  await waitFor("window.swapCalls === 1 && typeof window.finishSwap === 'function'");
  assert.equal(await evaluate("[...document.querySelectorAll('.page button, .page select')].every(b => b.disabled)"), true);
  await evaluate('window.finishSwap()');
  await waitFor("document.querySelector('.notice')?.dataset.kind === 'ok' && document.querySelector('.page').getAttribute('aria-busy') === 'false'");
  assert.equal(await evaluate('window.swapCalls'), 1);
  assert.equal(await evaluate("document.querySelector('.dlss-dll .dlss-ver').textContent"), '310.2.1.0');
  await evaluate("document.querySelector('.dlss-dll button[aria-label^=Restore]').click()");
  await waitFor("document.querySelector('.dlss-dll .dlss-ver').textContent === '3.7.10.0' && document.querySelector('.page').getAttribute('aria-busy') === 'false'");
  await evaluate("[...document.querySelectorAll('button')].find(b => b.textContent === 'Get a version…').click()");
  await waitFor("document.querySelector('.dlss-versions') !== null");
  await evaluate("window.dispatchEvent(new KeyboardEvent('keydown', {key: 'Escape', bubbles: true}))");
  await evaluate("[...document.querySelectorAll('button')].find(b => b.textContent === 'Get a version…').click()");
  await waitFor("document.querySelector('.dlss-versions') !== null");
  await evaluate("window.dispatchEvent(new KeyboardEvent('keydown', {key: 'Escape', bubbles: true})); window.mindos.call = window.originalCall");

  await clickNav('Updates');
  await evaluate("const b = [...document.querySelectorAll('button')].find(b => b.textContent === 'Update now'); b.focus(); b.click()");
  await waitFor("document.querySelector('[role=dialog]')?.contains(document.activeElement)");
  assert.equal(await evaluate("document.querySelector('.app-side').inert"), true);
  await evaluate("const b = document.querySelectorAll('.sheet button'); b[b.length - 1].focus(); window.dispatchEvent(new KeyboardEvent('keydown', {key: 'Tab', bubbles: true}))");
  assert.equal(await evaluate('document.activeElement.textContent'), 'Cancel');
  await evaluate("window.dispatchEvent(new KeyboardEvent('keydown', {key: 'Escape', bubbles: true}))");
  assert.equal(await evaluate("document.querySelector('.app-side').inert"), false);
  assert.equal(await evaluate('document.activeElement.textContent'), 'Update now');

  await clickNav('Keyboard & mouse');
  await waitFor("document.querySelector('.input-settings-fields')?.disabled === false");
  await evaluate("const s = document.querySelector('.page-input select'); s.value = 'de'; s.dispatchEvent(new Event('change')); document.querySelector('.page-input .primary').scrollIntoView(); document.querySelector('.page-input .primary').click()");
  await waitFor("document.querySelector('.notice')?.dataset.kind === 'ok' && !document.querySelector('fieldset').disabled");
  assert.equal(await evaluate("const n = document.querySelector('.page-input > .notice').getBoundingClientRect(); const c = document.querySelector('.app-content').getBoundingClientRect(); n.top >= c.top && n.bottom <= c.bottom"), true, 'Input feedback stays visible after applying at the bottom');
  await evaluate("[...document.querySelectorAll('.page-input button')].find(b => b.textContent === 'Choose defaults').click()");
  assert.equal(await evaluate("document.querySelector('.page-input select').value"), '');
  await evaluate("[...document.querySelectorAll('.page-input button')].find(b => b.textContent === 'Discard edits').click()");
  assert.equal(await evaluate("document.querySelector('.page-input select').value"), 'de');
  await evaluate(`window.inputOriginalCall = window.mindos.call;
    window.mindos.call = async function(method, params) {
      if (method === 'prefs.set') throw new Error('Simulated input rejection');
      return window.inputOriginalCall.call(this, method, params);
    };
    document.querySelector('.page-input select').value = 'fr';
    document.querySelector('.page-input .primary').click()`);
  await waitFor("document.querySelector('.notice')?.dataset.kind === 'error' && !document.querySelector('fieldset').disabled");
  await evaluate("[...document.querySelectorAll('.page-input button')].find(b => b.textContent === 'Discard edits').click(); window.mindos.call = window.inputOriginalCall");
  assert.equal(await evaluate("document.querySelector('.page-input select').value"), 'de');
  await clickNav('Overview');
  await clickNav('Keyboard & mouse');
  await waitFor("document.querySelector('.page-input select')?.value === 'de'");
  await shot('input');

  await send('Emulation.setDeviceMetricsOverride', { width: 720, height: 640, deviceScaleFactor: 1, mobile: false });
  await clickNav('Overview');
  await shot('overview-compact');
  assert.equal(await evaluate("document.querySelector('.app-content').scrollWidth <= document.querySelector('.app-content').clientWidth"), true);
  await clickNav('Performance');
  await shot('performance-compact');
  assert.equal(await evaluate("document.querySelector('.app-content').scrollWidth <= document.querySelector('.app-content').clientWidth"), true);
  await clickNav('Games');
  await shot('games-compact');
  assert.equal(await evaluate("document.querySelector('.app-content').scrollWidth <= document.querySelector('.app-content').clientWidth"), true);
  await clickNav('Keyboard & mouse');
  await shot('input-compact');
  assert.equal(await evaluate("document.querySelector('.app-content').scrollWidth <= document.querySelector('.app-content').clientWidth"), true);
  assert.deepEqual(errors, [], 'No uncaught browser exceptions');

  // Animated popups must clear the floating shelf, also in a scaled preview.
  await send('Emulation.setDeviceMetricsOverride', { width: 1280, height: 720, deviceScaleFactor: 1, mobile: false });
  await send('Page.navigate', { url: pathToFileURL(resolve(root, 'mindshell/ui/dist/index.html')).href + '?kind=preview&popup=notifications' });
  await waitFor("document.querySelector('.pop-notifications') !== null");
  await evaluate('document.fonts.ready');
  await delay(250);
  assert.equal(await evaluate("const p = document.querySelector('.pop-notifications').getBoundingClientRect(); const bar = document.querySelector('.panel-island').getBoundingClientRect(); p.bottom < bar.top && p.right <= innerWidth && p.top >= 0"), true, 'Notifications stay above the shelf after their entrance animation');
  await shot('notifications-shelf');
  assert.deepEqual(errors, [], 'No uncaught popup exceptions');
  console.log('PASS: native terminal launch/error feedback, navigation/search, mode/config feedback, software and gaming setup, game filtering/actions/rescan recovery, keyboard dialogs, compact layouts');
  console.log(`Screenshots: ${out}`);
} finally {
  socket?.close();
  chrome.kill('SIGTERM');
  if (chrome.exitCode === null) await once(chrome, 'exit');
  await rm(profile, { recursive: true, force: true, maxRetries: 5, retryDelay: 100 });
}
