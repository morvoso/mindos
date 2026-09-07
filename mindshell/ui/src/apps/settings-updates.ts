// Settings › Updates: what is waiting, what the Mind thinks of it, the last
// update and its snapshot, the health check, and every snapshot to go back to.

import * as bridge from '../bridge';
import { h } from '../dom';
import { icon } from '../icons';
import { noticeCard } from '../notices';
import { store } from '../state';
import type { HealthReport, RunResult, Snapshot, UpdateStatus } from '../types';
import { card, dialog, fmtDate, notice, pageHeader, pill, row, toggle } from './shared';

const request = <T = unknown>(req: Record<string, unknown>) => bridge.call<T>('mind.request', { request: req });

const TAG_LABEL: Record<string, string> = { kernel: 'Kernel', gpu: 'GPU', graphics: 'Graphics', core: 'Core', mindos: 'MindOS', gaming: 'Gaming' };

export function parseSnapshots(text: string): Snapshot[] {
  const out: Snapshot[] = [];
  for (const line of text.split('\n')) {
    const m = /^\s*(\d+)(\*?)\s+(\S+ \S+|\S+)\s+(\S+)\s+(\S+(?: \S+)?)\s{2,}(.*)$/.exec(line);
    if (!m) continue;
    out.push({ number: Number(m[1]), type: m[4], date: m[3], description: (m[2] ? '(booted) ' : '') + m[6].trim() });
  }
  return out;
}

export function updatesPage(el: HTMLElement, root: HTMLElement): () => void {
  const note = notice();
  const fail = (e: unknown) => note.show(`Mind: ${e instanceof Error ? e.message : String(e)}`, 'error');
  let snapshots: Snapshot[] = [];

  // ----- updates ------------------------------------------------------------
  const checkBtn = h('button', { class: 'btn' }, icon('refresh', 14), 'Check now');
  const applyBtn = h('button', { class: 'btn primary' }, icon('download', 14), 'Update now');
  const autoToggle = toggle(false, (v) => request({ type: 'set_auto_update', enabled: v }).then(() => note.show(v ? 'Low-risk updates will be applied on their own, never while a game runs.' : 'Updates wait for you.', 'ok')).catch(fail));
  const summary = h('div', { class: 'upd-summary' });
  const pkgs = h('div', { class: 'upd-pkgs' });
  const news = h('div', { class: 'upd-news' });
  const updCard = card('Updates', summary, pkgs, news, h('div', { class: 'card-actions' }, checkBtn, applyBtn, h('span', { class: 'strip-gap' })), row('Apply low-risk updates automatically', 'The Mind checks every few hours. When nothing touches the kernel, the graphics driver or the core system, and no game is running, it updates on its own and verifies the system after.', autoToggle));

  checkBtn.addEventListener('click', () => {
    checkBtn.classList.add('busy');
    request({ type: 'updates', check: true }).catch(fail).finally(() => checkBtn.classList.remove('busy'));
  });
  applyBtn.addEventListener('click', () => {
    const u = store.state.mind?.updates;
    const body = h('div', { class: 'stack' }, h('p', {}, `Install ${u?.packages.length ?? 0} package update${u?.packages.length === 1 ? '' : 's'}? A snapshot is taken first; the Mind checks the system after and tells you if something is off.`), u?.warnings.length ? h('ul', { class: 'upd-warnings' }, ...u.warnings.map((w) => h('li', {}, w))) : null);
    const close = dialog(root, 'UPDATE NOW', body, [h('button', { class: 'btn', onclick: () => close() }, 'Cancel'), h('button', { class: 'btn primary', onclick: () => { close(); request({ type: 'apply_updates' }).then(() => note.show('Updating…', 'ok')).catch(fail); } }, 'Update')]);
  });

  // ----- last update ------------------------------------------------------------
  const lastBody = h('div', { class: 'stack' });
  const lastCard = card('After the last update', lastBody);

  // ----- health -------------------------------------------------------------------
  const healthBtn = h('button', { class: 'btn' }, icon('pulse', 14), 'Check now');
  const healthBody = h('div', { class: 'ntc-list' });
  const healthLine = h('div', { class: 'row-help' });
  const healthCard = card('Health', h('p', { class: 'card-help' }, 'The Mind looks at failed services, the kernel and driver, disk space, pacnew files and the boot snapshots — after every update and every half hour.'), healthLine, healthBody, h('div', { class: 'card-actions' }, healthBtn));
  healthBtn.addEventListener('click', () => {
    healthBtn.classList.add('busy');
    request<HealthReport>({ type: 'health' })
      .then((r) => {
        if (store.state.mind) store.state.mind.health = r;
        renderHealth();
      })
      .catch(fail)
      .finally(() => healthBtn.classList.remove('busy'));
  });

  // ----- snapshots ------------------------------------------------------------------
  const snapBody = h('div', { class: 'list' });
  const snapCard = card('Snapshots', h('p', { class: 'card-help' }, 'Every update takes a snapshot first. Any of them can be booted from the boot menu, or made the system again from here; the current state is kept so a rollback can be undone.'), snapBody);

  el.append(pageHeader('Updates', 'The Mind watches for updates, reads the Arch news, and tells you when something could break a game or the desktop.'), note.el, updCard, lastCard, healthCard, snapCard);

  const rollback = (n: number, why: string) => {
    const body = h('div', { class: 'stack' }, h('p', {}, `Make snapshot ${n} the system again${why ? ` (${why})` : ''}? The machine reboots into it. What you have now is kept as a new snapshot.`));
    const close = dialog(root, 'ROLL BACK', body, [h('button', { class: 'btn', onclick: () => close() }, 'Cancel'), h('button', { class: 'btn danger', onclick: () => { close(); request({ type: 'rollback', snapshot: n }).then(() => note.show(`Rolling back to snapshot ${n}; rebooting.`, 'ok')).catch(fail); } }, 'Roll back and reboot')]);
  };

  const renderUpdates = () => {
    const u: UpdateStatus | null | undefined = store.state.mind?.updates;
    const at = autoToggle.querySelector('input') as HTMLInputElement;
    if (u && document.activeElement !== at) at.checked = !!u.auto_apply;
    summary.replaceChildren();
    pkgs.replaceChildren();
    news.replaceChildren();
    if (!u) {
      summary.appendChild(h('div', { class: 'row-help' }, store.state.mind?.daemon ? 'No check yet.' : 'The Mind is not running.'));
      applyBtn.disabled = true;
      return;
    }
    const n = u.packages.length;
    const head = h('div', { class: 'upd-head' });
    if (u.checking) head.append(pill('Checking…', 'accent'));
    else if (u.applying) head.append(pill('Updating…', 'accent'));
    else if (u.assessing) head.append(pill('Mind is reading the changes…', 'mind'));
    else if (n === 0) head.append(pill('Up to date', 'ok'));
    else head.append(pill(`${n} update${n === 1 ? '' : 's'}`, 'accent'));
    if (n && u.risk) head.append(pill(`${u.risk} risk`, u.risk === 'high' ? 'danger' : u.risk === 'medium' ? 'warn' : 'ok'));
    if (u.reboot) head.append(pill('Reboot after', 'warn'));
    if (u.manual_intervention) head.append(pill('Manual steps in the news', 'danger'));
    head.append(h('span', { class: 'strip-gap' }), h('span', { class: 'row-help' }, u.checked_at ? `Checked ${fmtDate(u.checked_at)}` : 'Never checked'));
    summary.appendChild(head);
    if (u.error) summary.appendChild(h('div', { class: 'row-help danger' }, u.error));
    if (u.summary) summary.appendChild(h('div', { class: 'upd-text' }, h('span', { class: 'upd-by' }, icon(u.assessed_by_model ? 'mind' : 'shield', 13), u.assessed_by_model ? 'The Mind says' : 'By the rules'), u.summary));
    if (u.warnings.length) summary.appendChild(h('ul', { class: 'upd-warnings' }, ...u.warnings.map((w) => h('li', {}, icon('warning', 13), w))));
    applyBtn.disabled = n === 0 || u.applying || u.checking;
    checkBtn.disabled = u.checking || u.applying;
    if (n) {
      const rows = u.packages.map((p) => h('div', { class: 'upd-pkg' }, h('span', { class: 'upd-name mono' }, p.name), h('span', { class: 'upd-ver mono' }, `${p.from} → ${p.to}`), p.tag ? pill(TAG_LABEL[p.tag] ?? p.tag, p.tag === 'kernel' || p.tag === 'gpu' ? 'warn' : p.tag === 'core' ? 'danger' : '') : h('span')));
      const details = h('details', { class: 'upd-details' }, h('summary', {}, `Packages (${n})`), h('div', { class: 'upd-list' }, ...rows));
      if (n <= 12) details.open = true;
      pkgs.appendChild(details);
    }
    if (u.news.length) {
      news.appendChild(h('details', { class: 'upd-details' }, h('summary', {}, `Arch news (${u.news.length})`), h('div', { class: 'upd-list' }, ...u.news.slice(0, 8).map((x) => h('button', { class: 'linkish upd-newsitem', onclick: () => bridge.send('shell.exec', { cmd: `xdg-open ${x.url}` }) }, h('span', { class: 'mono' }, x.date), x.title, icon('external', 11))))));
    }
  };

  const renderLast = () => {
    const l = store.state.mind?.updates?.last_update;
    lastBody.replaceChildren();
    if (!l) {
      lastBody.appendChild(h('div', { class: 'row-help' }, 'No update recorded yet.'));
      return;
    }
    const head = h('div', { class: 'upd-head' }, l.ok ? pill('Installed', 'ok') : pill('Failed', 'danger'), l.verified === 'ok' ? pill('Verified', 'ok') : l.verified === 'problems' ? pill('Problems found', 'danger') : pill('Not verified yet', ''), h('span', { class: 'strip-gap' }), h('span', { class: 'row-help' }, `${fmtDate(l.time)} · ${l.packages.length} package${l.packages.length === 1 ? '' : 's'}`));
    lastBody.appendChild(head);
    if (l.report) lastBody.appendChild(h('div', { class: 'upd-text' }, l.report));
    if (l.packages.length) lastBody.appendChild(h('div', { class: 'row-help mono wrap' }, l.packages.slice(0, 40).join('  ') + (l.packages.length > 40 ? ' …' : '')));
    if (l.pre_snapshot) {
      lastBody.appendChild(
        row(`Snapshot ${l.pre_snapshot} from before`, 'The system exactly as it was before this update.', h('button', { class: 'btn danger small', onclick: () => rollback(l.pre_snapshot!, 'before the last update') }, icon('history', 13), 'Roll back')),
      );
    }
  };

  const renderHealth = () => {
    const hr = store.state.mind?.health;
    healthBody.replaceChildren();
    if (!hr) {
      healthLine.textContent = 'No check yet.';
      return;
    }
    const bad = hr.findings.filter((f) => f.level === 'warn' || f.level === 'danger');
    healthLine.replaceChildren(bad.length ? pill(`${bad.length} thing${bad.length === 1 ? '' : 's'} to look at`, 'warn') : pill('All good', 'ok'), ' ', h('span', {}, `Checked ${fmtDate(hr.checked_at)}`));
    for (const f of hr.findings) {
      if (f.level === 'ok' && bad.length) continue;
      healthBody.appendChild(noticeCard(f, { compact: true }));
    }
  };

  const renderSnapshots = () => {
    snapBody.replaceChildren();
    if (!snapshots.length) {
      snapBody.appendChild(h('div', { class: 'row-help' }, 'No snapshots yet (snapper takes one at every update).'));
      return;
    }
    for (const s of snapshots.slice(0, 12)) {
      snapBody.appendChild(row(`#${s.number} · ${s.description || s.type}`, `${s.date} · ${s.type}`, h('button', { class: 'btn small', onclick: () => rollback(s.number, s.description) }, icon('history', 13), 'Restore')));
    }
  };

  const loadSnapshots = () => {
    bridge
      .call<RunResult>('shell.run', { argv: ['mindos-boot', 'list'] })
      .then((r) => {
        snapshots = parseSnapshots(r.stdout);
        renderSnapshots();
      })
      .catch(() => renderSnapshots());
  };

  const renderAll = () => {
    renderUpdates();
    renderLast();
    renderHealth();
  };
  renderAll();
  renderSnapshots();
  loadSnapshots();
  store.bind(el, 'mindUpdates', () => {
    renderUpdates();
    renderLast();
  });
  store.bind(el, 'mindHealth', renderHealth);
  store.bind(el, 'mind', renderAll);
  if (!store.state.mind?.updates) {
    request<UpdateStatus>({ type: 'updates', check: false })
      .then((u) => {
        if (store.state.mind) store.state.mind.updates = u;
        renderUpdates();
        renderLast();
      })
      .catch(() => undefined);
  }
  return () => undefined;
}
