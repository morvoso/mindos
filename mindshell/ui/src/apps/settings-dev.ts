// Settings › Developer. Reports the toolchains detected on the system and
// manages the steps a package cannot perform: per-user setup, services,
// group membership and SSH keys.

import * as bridge from '../bridge';
import { h } from '../dom';
import { icon } from '../icons';
import { store } from '../state';
import type { DevStatus, DevTool, RunResult } from '../types';
import { card, dialog, notice, pageHeader, pill, row, selectBox } from './shared';

// Toolchains offered by the Install button. Every entry is an official Arch
// package and the install runs with --repo-only, so the AUR is never used.
// Must match DEV_PACKAGES in mindshell/src/app.rs, which the host enforces.
const CATALOGUE: { label: string; pkgs: string[] }[] = [
  { label: '.NET SDK', pkgs: ['dotnet-sdk'] },
  { label: 'Bun', pkgs: ['bun'] },
  { label: 'Crystal', pkgs: ['crystal'] },
  { label: 'Dart', pkgs: ['dart'] },
  { label: 'Deno', pkgs: ['deno'] },
  { label: 'Elixir', pkgs: ['elixir'] },
  { label: 'Go', pkgs: ['go'] },
  { label: 'Haskell (GHC)', pkgs: ['ghc'] },
  { label: 'Java (OpenJDK)', pkgs: ['jdk-openjdk'] },
  { label: 'Julia', pkgs: ['julia'] },
  { label: 'Kotlin', pkgs: ['kotlin'] },
  { label: 'Lua', pkgs: ['lua'] },
  { label: 'Node.js', pkgs: ['nodejs', 'npm'] },
  { label: 'PHP', pkgs: ['php'] },
  { label: 'Python', pkgs: ['python', 'python-pip'] },
  { label: 'R', pkgs: ['r'] },
  { label: 'Ruby', pkgs: ['ruby'] },
  { label: 'Rust (rustup)', pkgs: ['rustup'] },
  { label: 'Zig', pkgs: ['zig'] },
  { label: 'Podman + distrobox', pkgs: ['podman', 'distrobox'] },
];

const TIPS: [string, string][] = [
  ['Super + Return', 'Opens a terminal (foot) with fish, starship and zoxide configured'],
  ['mindos-pkg install NAME', 'Installs a package from the repositories or Flathub. Add --aur to permit the AUR.'],
  ['distrobox create -i ubuntu:24.04', "Creates a container running another distribution, with access to the home directory"],
  ['lazygit', 'Terminal interface for git. Diffs are rendered by delta and difftastic.'],
  ['just / mold / sccache', 'Task runner, linker and compilation cache'],
  ['perf / gdb / lldb / strace / hyperfine', 'Profiling, debugging, tracing and benchmarking'],
  ['man / tldr NAME', 'Manual pages and condensed command summaries'],
];

export function devPage(el: HTMLElement, root: HTMLElement): () => void {
  const note = notice();
  let status: DevStatus | undefined;

  // pkexec exits 126 when the authentication dialog is dismissed (127: could not run).
  const cancelled = (r: RunResult) => r.status === 126 && !r.stderr.trim();

  /** Run a helper from a button, report the outcome and refresh the page. */
  const action = (btn: HTMLButtonElement, argv: string[], done: string, onOk?: (r: RunResult) => string) => {
    btn.classList.add('busy');
    btn.disabled = true;
    bridge
      .call<RunResult>('shell.run', { argv })
      .then((r) => {
        if (r.ok) note.show(onOk ? onOk(r) : done, 'ok');
        else if (cancelled(r)) note.show('Cancelled.', 'info');
        else note.show(`${argv[0] === 'pkexec' ? argv[1] : argv[0]} failed: ${r.stderr.trim() || r.stdout.trim() || `exit ${r.status}`}`, 'error');
      })
      .catch((e) => note.show(e instanceof Error ? e.message : String(e), 'error'))
      .finally(() => {
        btn.classList.remove('busy');
        btn.disabled = false;
        void refresh();
      });
  };

  /** A small button that runs argv. Privileged commands go through pkexec,
   *  which opens the shell's authentication dialog. */
  const doBtn = (label: string, argv: string[], done: string, kind = ''): HTMLButtonElement => {
    const btn = h('button', { class: `btn small${kind ? ` ${kind}` : ''}` }, label) as HTMLButtonElement;
    btn.addEventListener('click', () => action(btn, argv, done));
    return btn;
  };

  // ---- toolchains ----------------------------------------------------------
  const chainsBody = h('div', { class: 'dev-tools' });
  const toolsBody = h('div', { class: 'dev-tools' });
  const addSel = selectBox(
    CATALOGUE.map((c) => ({ value: c.label, label: c.label })),
    CATALOGUE[0].label,
    () => undefined,
  );
  const addBtn = h('button', { class: 'btn small accent' }, icon('plus', 13), 'Install') as HTMLButtonElement;
  addBtn.addEventListener('click', () => {
    const entry = CATALOGUE.find((c) => c.label === addSel.value);
    if (!entry) return;
    action(addBtn, ['pkexec', 'mindos-pkg', 'install', '--repo-only', ...entry.pkgs], `${entry.label} installed.`);
  });
  const setupBtn = h('button', { class: 'btn small' }, icon('wrench', 13), 'Finish setup') as HTMLButtonElement;
  setupBtn.addEventListener('click', () =>
    action(setupBtn, ['mindos-dev-setup'], 'Setup complete.', (r) => {
      const lines = (r.stdout + r.stderr).replace(/\x1b\[[0-9;]*m/g, '').split('\n').filter((l) => /^[✓!]/.test(l));
      return lines.length ? lines.join(' · ') : 'No changes were needed.';
    }),
  );
  const chainsCard = card(
    'Toolchains',
    h('p', { class: 'card-help' }, 'Language toolchains detected on this system. Install additional toolchains from the list below, or with mindos-pkg install NAME in a terminal.'),
    chainsBody,
    h('div', { class: 'card-actions' }, addSel, addBtn, h('span', { class: 'row-help' }, 'Installs from the official Arch repositories only. The AUR is not used.')),
    h('div', { class: 'card-actions' }, setupBtn, h('span', { class: 'row-help' }, 'Completes the per-user setup: the Rust toolchain (when rustup is installed), editor defaults and the git identity.')),
  );
  const toolsCard = card('Build and debug tools', h('p', { class: 'card-help' }, 'Installed by the mindos-dev package.'), toolsBody);

  // ---- containers ----------------------------------------------------------
  const containersBody = h('div', { class: 'list' });
  const containersCard = card('Containers', containersBody);

  // ---- ssh -----------------------------------------------------------------
  const sshBody = h('div', { class: 'list' });
  const sshCard = card('SSH', sshBody);

  const showKey = (path: string) => {
    bridge
      .call<RunResult>('shell.run', { argv: ['cat', path] })
      .then((r) => {
        if (!r.ok) return note.show(`Could not read ${path}.`, 'error');
        const text = r.stdout.trim();
        const box = h('textarea', { class: 'keybox mono', readonly: true, rows: 3 }) as HTMLTextAreaElement;
        box.value = text;
        const copy = h('button', { class: 'btn small accent' }, 'Copy') as HTMLButtonElement;
        const close = dialog(root, 'Public key', h('div', {}, h('p', { class: 'card-help' }, 'Add this key to a git hosting account, or to ~/.ssh/authorized_keys on a remote system. The private key stays on this computer.'), box), [
          copy,
          h('button', { class: 'btn small', onclick: () => close() }, 'Close'),
        ]);
        copy.addEventListener('click', () => {
          box.select();
          navigator.clipboard?.writeText(text).then(
            () => note.show('Public key copied.', 'ok'),
            () => note.show('Clipboard unavailable. Select the text and press Ctrl+C.', 'info'),
          );
        });
      })
      .catch((e) => note.show(e instanceof Error ? e.message : String(e), 'error'));
  };

  // ---- access --------------------------------------------------------------
  const groupsBody = h('div', { class: 'list' });
  const limitsBody = h('div', { class: 'dev-tools' });
  const accessCard = card(
    'Access',
    h('p', { class: 'card-help' }, 'Group membership grants access to devices and services without sudo. Changes take effect at the next login.'),
    groupsBody,
    h('h3', { class: 'card-sub' }, 'Kernel limits'),
    h('p', { class: 'card-help' }, 'Set by mindos-dev in /usr/lib/sysctl.d/61-mindos-dev.conf. To change a value, add a higher-numbered file in /etc/sysctl.d.'),
    limitsBody,
  );

  // ---- git -----------------------------------------------------------------
  const gitName = h('input', { class: 'input', type: 'text', placeholder: 'Your name' }) as HTMLInputElement;
  const gitEmail = h('input', { class: 'input', type: 'email', placeholder: 'you@example.com' }) as HTMLInputElement;
  const gitSave = h('button', { class: 'btn small accent' }, 'Save') as HTMLButtonElement;
  gitSave.addEventListener('click', () => {
    const name = gitName.value.trim();
    const email = gitEmail.value.trim();
    if (!name || !email) return note.show('Enter a name and an email address.', 'error');
    gitSave.disabled = true;
    Promise.all([
      bridge.call<RunResult>('shell.run', { argv: ['git', 'config', '--global', 'user.name', name] }),
      bridge.call<RunResult>('shell.run', { argv: ['git', 'config', '--global', 'user.email', email] }),
    ])
      .then(([a, b]) => note.show(a.ok && b.ok ? `Git identity set to ${name} <${email}>.` : 'Could not write ~/.gitconfig.', a.ok && b.ok ? 'ok' : 'error'))
      .catch((e) => note.show(e instanceof Error ? e.message : String(e), 'error'))
      .finally(() => {
        gitSave.disabled = false;
        void refresh();
      });
  });
  const gitCard = card(
    'Git identity',
    h('p', { class: 'card-help' }, 'The name and email address recorded on commits, stored in ~/.gitconfig. Other git defaults are set in /etc/mindos/dev/gitconfig.'),
    h('div', { class: 'field-row' }, gitName, gitEmail, gitSave),
  );

  const tipsCard = card('Reference', h('table', { class: 'keys' }, ...TIPS.map(([k, v]) => h('tr', {}, h('td', { class: 'key mono' }, k), h('td', {}, v)))));
  const docsBtn = h('button', { class: 'btn' }, icon('external', 14), 'Open docs/DEV.md');
  docsBtn.addEventListener('click', () => bridge.send('shell.exec', { cmd: 'xdg-open /usr/share/doc/mindos/DEV.md' }));

  el.append(
    pageHeader('Developer', 'Toolchains, containers, SSH and system access. This page shows what is installed and manages the settings that require administrator rights.'),
    note.el,
    chainsCard,
    containersCard,
    sshCard,
    accessCard,
    gitCard,
    toolsCard,
    tipsCard,
    h('div', { class: 'card-actions' }, docsBtn),
  );

  const tool = (t: DevTool) => h('div', { class: 'dev-tool' }, h('span', { class: 'dev-name' }, t.name), h('span', { class: 'dev-ver mono' }, t.version));

  const render = () => {
    if (!status) {
      chainsBody.replaceChildren(h('div', { class: 'row-help' }, 'mindos-dev-setup did not respond. Check that the mindos-dev package is installed.'));
      for (const b of [toolsBody, containersBody, sshBody, groupsBody, limitsBody]) b.replaceChildren();
      return;
    }
    chainsBody.replaceChildren(...(status.toolchains.length ? status.toolchains.map(tool) : [h('div', { class: 'row-help' }, 'No language toolchains detected. Select one below, or install one with mindos-pkg.')]));
    toolsBody.replaceChildren(...status.tools.map(tool));

    const d = status.docker;
    containersBody.replaceChildren(
      d.installed
        ? row(
            'Docker service',
            d.active ? 'Running.' : d.enabled ? 'Enabled. Starts on demand.' : 'Not running. Required to run Docker containers.',
            d.active ? pill('Running', 'ok') : d.enabled ? pill('Enabled', '') : doBtn('Enable', ['pkexec', 'systemctl', 'enable', '--now', 'docker.service'], 'Docker service enabled.', 'accent'),
          )
        : row('Docker', 'Not installed. Podman provides rootless containers without a service.', pill('Not installed', '')),
      status.podman.installed
        ? row('Podman', 'Installed. Runs containers without root access or a background service. Used by distrobox.', pill('Ready', 'ok'))
        : row('Podman', 'Not installed. Provides rootless containers and is required by distrobox.', doBtn('Install', ['pkexec', 'mindos-pkg', 'install', '--repo-only', 'podman', 'distrobox'], 'Podman and distrobox installed.')),
    );

    const s = status.ssh;
    const key = s.keys[0];
    sshBody.replaceChildren(
      row(
        'SSH key',
        key ? `${key.type} key in ~/.ssh (${key.comment}). Used for SSH logins and git remotes.` : 'No SSH key found. A key is required for SSH logins and git remotes over SSH.',
        key
          ? h('button', { class: 'btn small', onclick: () => showKey(key.path) }, 'Show public key')
          : doBtn('Create a key', ['ssh-keygen', '-t', 'ed25519', '-N', '', '-C', `${status.user}@${store.state.host}`, '-f', s.new_key], `SSH key created at ${s.new_key}.`, 'accent'),
      ),
      s.installed
        ? row(
            'SSH server',
            s.active
              ? `Running. This system accepts SSH connections on port ${s.port ?? 22}.`
              : 'Not running. When enabled, other systems can log in to this one over SSH. Enable it only on trusted networks.',
            s.active ? doBtn('Disable', ['pkexec', 'systemctl', 'disable', '--now', 'sshd.service'], 'SSH server disabled.') : doBtn('Enable', ['pkexec', 'systemctl', 'enable', '--now', 'sshd.service'], 'SSH server enabled.'),
            s.active ? pill('Listening', 'warn') : null,
          )
        : row('SSH server', 'openssh is not installed. Remote login over SSH is not available.', doBtn('Install openssh', ['pkexec', 'mindos-pkg', 'install', '--repo-only', 'openssh'], 'openssh installed.')),
    );

    groupsBody.replaceChildren(
      ...status.groups.map((g) =>
        row(
          g.name,
          g.member ? g.help : `${g.help}. Not a member of this group.`,
          g.member ? pill('Member', 'ok') : doBtn('Join', ['pkexec', 'usermod', '-aG', g.name, status!.user], `Added to the ${g.name} group. Log out and back in to apply.`, 'accent'),
        ),
      ),
    );

    const l = status.limits;
    const lim = (name: string, v: number | null) => h('div', { class: 'dev-tool' }, h('span', { class: 'dev-name' }, name), h('span', { class: 'dev-ver mono' }, v === null ? 'unknown' : v.toLocaleString()));
    limitsBody.replaceChildren(
      lim('inotify watches per user', l.inotify_watches),
      lim('inotify instances per user', l.inotify_instances),
      lim('perf_event_paranoid', l.perf_paranoid),
    );

    if (document.activeElement !== gitName) gitName.value = status.git.name ?? '';
    if (document.activeElement !== gitEmail) gitEmail.value = status.git.email ?? '';
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
