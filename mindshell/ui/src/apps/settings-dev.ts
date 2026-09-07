// Settings › Developer: the developer stack MindOS ships, and the bits
// that need a hand (Docker group membership, the shell prompt).

import * as bridge from '../bridge';
import { h } from '../dom';
import { icon } from '../icons';
import { store } from '../state';
import type { DevStatus, RunResult } from '../types';
import { card, notice, pageHeader, pill, row } from './shared';

const TIPS: [string, string][] = [
  ['Super + Return', 'A terminal (foot) with fish, starship and zoxide ready'],
  ['distrobox create -i ubuntu:24.04', 'A full Ubuntu userland inside a container, sharing your home'],
  ['docker run hello-world', 'Docker is installed; the user must be in the docker group'],
  ['lazygit', 'Git in the terminal; delta shows the diffs'],
  ['just / mold / sccache', 'Task runner, fast linker, compile cache — set up for Rust and C'],
  ['uv / go / node / python', 'Toolchains for the usual languages'],
  ['perf / gdb / strace / hyperfine', 'Profiling, debugging and benchmarking'],
];

export function devPage(el: HTMLElement): () => void {
  const note = notice();
  let status: DevStatus | undefined;
  const toolsBody = h('div', { class: 'dev-tools' });
  const setupBtn = h('button', { class: 'btn' }, icon('wrench', 14), 'Set up toolchains');
  const setupHelp = h('span', { class: 'row-help' }, 'Installs the Rust stable toolchain for you, the shell prompt and the Docker bits that are missing.');
  const toolsCard = card('Tools', h('p', { class: 'card-help' }, 'From mindos-dev. The version shown is what is installed right now.'), toolsBody, h('div', { class: 'card-actions' }, setupBtn, setupHelp));
  setupBtn.addEventListener('click', () => {
    setupBtn.classList.add('busy');
    setupBtn.disabled = true;
    bridge
      .call<RunResult>('shell.run', { argv: ['mindos-dev-setup'] })
      .then((r) => {
        const lines = (r.stdout + r.stderr).replace(/\x1b\[[0-9;]*m/g, '').split('\n').filter((l) => /^[✓!]/.test(l));
        note.show(r.ok ? (lines.length ? lines.join(' · ') : 'Everything was already set up.') : `mindos-dev-setup failed: ${r.stderr.trim() || r.stdout.trim()}`, r.ok ? 'ok' : 'error');
      })
      .catch((e) => note.show(e instanceof Error ? e.message : String(e), 'error'))
      .finally(() => {
        setupBtn.classList.remove('busy');
        setupBtn.disabled = false;
        void refresh();
      });
  });
  const dockerBody = h('div', { class: 'list' });
  const dockerCard = card('Docker', dockerBody);
  const tipsBody = h('table', { class: 'keys' }, ...TIPS.map(([k, v]) => h('tr', {}, h('td', { class: 'key mono' }, k), h('td', {}, v))));
  const tipsCard = card('Where things are', tipsBody);
  const docsBtn = h('button', { class: 'btn' }, icon('external', 14), 'Open docs/DEV.md');
  docsBtn.addEventListener('click', () => bridge.send('shell.exec', { cmd: 'xdg-open /usr/share/doc/mindos/DEV.md' }));
  el.append(pageHeader('Developer', 'MindOS is a workstation too: the compilers, containers and CLI tools are installed, the shell is set up, and the kernel keeps its tracing bits.'), note.el, toolsCard, dockerCard, tipsCard, h('div', { class: 'card-actions' }, docsBtn));

  const render = () => {
    toolsBody.replaceChildren();
    if (!status) {
      toolsBody.appendChild(h('div', { class: 'row-help' }, 'mindos-dev-setup is not answering; is mindos-dev installed?'));
      dockerBody.replaceChildren();
      return;
    }
    for (const t of status.tools) {
      toolsBody.appendChild(h('div', { class: `dev-tool${t.version ? '' : ' missing'}` }, h('span', { class: 'dev-name mono' }, t.name), h('span', { class: 'dev-ver mono' }, t.version ?? 'not installed')));
    }
    const d = status.docker;
    dockerBody.replaceChildren(
      row('Service', d.active ? 'Running.' : d.enabled ? 'Enabled, starts on demand.' : 'Not running.', d.active ? pill('Running', 'ok') : d.enabled ? pill('Enabled', '') : h('button', { class: 'btn small', onclick: () => bridge.send('shell.exec', { cmd: 'sudo systemctl enable --now docker.service; sleep 1', terminal: true }) }, 'Enable')),
      row(`${store.state.user} in the docker group`, d.member ? 'Containers run without sudo.' : 'Needed to run containers without sudo. Log out and in again after joining.', d.member ? pill('Yes', 'ok') : h('button', { class: 'btn small accent', onclick: () => bridge.send('shell.exec', { cmd: `sudo usermod -aG docker ${store.state.user}`, terminal: true }) }, 'Join')),
    );
  };
  const refresh = () =>
    bridge
      .call<RunResult>('shell.run', { argv: ['mindos-dev-setup', '--status', '--json'] })
      .then((r) => {
        status = r.ok ? (r.json as DevStatus) : undefined;
        render();
      })
      .catch(() => {
        status = undefined;
        render();
      });
  render();
  void refresh();
  return () => undefined;
}
