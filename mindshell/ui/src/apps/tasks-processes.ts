// The process table: everything running, sorted how you like, and the button
// that ends it.
//
// The sorting, the filtering and the cut to a screenful all happen in the
// host, not here: a machine with a thousand processes should not send nine
// hundred of them across the bridge for JavaScript to throw away. What arrives
// is the page you are looking at.

import * as bridge from '../bridge';
import { anchored, debounce, every, h } from '../dom';
import { icon } from '../icons';
import { bytes, count, duration, heat, rate } from '../monitor';
import type { ProcessDetail, ProcessRow, ProcessTable } from '../types';
import { dataTable, factGrid } from './tasks-parts';
import { dialog, notice, pageHeader } from './shared';

const TICK = 2000;

/** What ending a process actually does, in the order of how hard it is. */
const SIGNALS: { id: string; label: string; help: string }[] = [
  { id: 'TERM', label: 'End task', help: 'Asks the program to close, so it can save first.' },
  { id: 'KILL', label: 'Force end', help: 'Stops it immediately. Anything unsaved is lost.' },
  { id: 'STOP', label: 'Pause', help: 'Freezes it until it is resumed.' },
  { id: 'CONT', label: 'Resume', help: 'Continues a paused process.' },
];

export function processesPage(el: HTMLElement, root: HTMLElement): () => void {
  const head = pageHeader('Processes', 'Reading the table…');
  const sub = head.querySelector<HTMLElement>('.page-sub')!;
  const status = notice();

  let sort = { key: 'cpu', ascending: false };
  let query = '';
  let mine = false;
  let cores = 1;
  let paused = false;

  const search = h('input', {
    type: 'search',
    class: 'tk-search',
    placeholder: 'Filter by name, user or PID',
    'aria-label': 'Filter processes',
    autocomplete: 'off',
    spellcheck: false,
  }) as HTMLInputElement;
  const mineToggle = h('button', { class: 'btn', type: 'button', 'aria-pressed': 'false' }, icon('user', 14), 'My processes');
  const pauseToggle = h('button', { class: 'btn', type: 'button', 'aria-pressed': 'false' }, icon('clock', 14), 'Freeze list');
  const endButton = h('button', { class: 'btn danger', type: 'button', disabled: true }, icon('x', 14), 'End task') as HTMLButtonElement;
  const toolbar = h('div', { class: 'tk-toolbar' }, h('div', { class: 'tk-search-box' }, icon('search', 14), search), mineToggle, pauseToggle, h('span', { class: 'tk-toolbar-gap' }), endButton);

  const table = dataTable<ProcessRow>(
    [
      {
        key: 'name',
        label: 'Name',
        width: 'minmax(0,2.2fr)',
        sortable: true,
        cell: (p) => p.name,
        // Four hundred rows, twice a second: the name and its badges are
        // written into the cell that is already there, and the badges are
        // rebuilt only when a process actually changes what it is.
        render: (cell, p) => {
          let node = cell.firstElementChild as HTMLElement | null;
          if (!node) {
            node = h('span', { class: 'tk-proc' }, h('b', {}, ''));
            cell.replaceChildren(node);
          }
          const name = node.firstElementChild as HTMLElement;
          if (name.textContent !== p.name) name.textContent = p.name;
          const badges = [
            p.wine ? ['Windows', ''] : null,
            p.state === 'Z' ? ['Zombie', ' warn'] : null,
            p.state === 'T' ? ['Paused', ''] : null,
          ].filter(Boolean) as [string, string][];
          const mark = badges.map(([label]) => label).join(',');
          if (node.dataset.badges !== mark) {
            node.dataset.badges = mark;
            while (node.children.length > 1) node.lastElementChild!.remove();
            for (const [label, tone] of badges) node.append(h('span', { class: `tk-badge${tone}` }, label));
          }
          const title = p.cmd || p.name;
          if (node.title !== title) node.title = title;
        },
      },
      { key: 'pid', label: 'PID', width: '72px', align: 'end', sortable: true, cell: (p) => String(p.pid) },
      { key: 'user', label: 'User', width: 'minmax(0,1fr)', sortable: true, cell: (p) => p.user ?? '' },
      { key: 'cpu', label: 'CPU', width: '76px', align: 'end', sortable: true, cell: (p) => `${p.cpu.toFixed(1)}%`, tone: (p) => heat(cores > 0 ? p.cpu / cores : p.cpu) },
      { key: 'mem', label: 'Memory', width: '92px', align: 'end', sortable: true, cell: (p) => bytes(p.rss) },
      { key: 'disk', label: 'Disk', width: '92px', align: 'end', sortable: true, cell: (p) => rate((p.readRate ?? 0) + (p.writeRate ?? 0)) },
      { key: 'threads', label: 'Threads', width: '78px', align: 'end', sortable: true, cell: (p) => String(p.threads) },
    ],
    (p) => String(p.pid),
    {
      sort,
      empty: 'Nothing matches that filter.',
      onSort: (key) => {
        sort = sort.key === key ? { key, ascending: !sort.ascending } : { key, ascending: key === 'name' || key === 'user' };
        table.setSort(sort);
        void refresh();
      },
      onSelect: (row) => {
        endButton.disabled = !row;
      },
      onActivate: (row) => showDetail(row),
      onContext: (row, at) => menu(row, at),
    },
  );

  el.append(head, status.el, toolbar, table.el);

  // ---- the table itself

  let busy = false;
  const refresh = async () => {
    if (busy || paused || el.offsetParent === null) return;
    busy = true;
    try {
      const answer = await bridge.call<ProcessTable>('system.processes', {
        query,
        sort: sort.key,
        order: sort.ascending ? 'asc' : 'desc',
        mine,
        limit: 400,
      });
      if (!el.isConnected) return;
      // Processes come and go between ticks; the row you were reading should
      // not move because one above it exited.
      anchored(el, () => {
        cores = answer.cores || 1;
        table.set(answer.processes);
        const shown = answer.processes.length;
        const running = answer.states['R'] ?? 0;
        const sleeping = answer.states['S'] ?? 0;
        const zombies = answer.states['Z'] ?? 0;
        sub.textContent = [
          `${count(answer.total)} processes`,
          `${count(answer.threads)} threads`,
          `${running} running · ${sleeping} sleeping`,
          zombies ? `${zombies} zombie${zombies === 1 ? '' : 's'}` : '',
          query || mine ? `${answer.matched} matching, ${shown} shown` : shown < answer.total ? `${shown} shown` : '',
        ]
          .filter(Boolean)
          .join(' · ');
      });
    } catch (e) {
      if (el.isConnected) status.show(bridge.reason(e), 'error');
    } finally {
      busy = false;
    }
  };

  search.addEventListener(
    'input',
    debounce(() => {
      query = search.value;
      void refresh();
    }, 200),
  );
  search.addEventListener('keydown', (e) => {
    if (e.key === 'Escape' && search.value) {
      e.stopPropagation();
      search.value = '';
      query = '';
      void refresh();
    }
  });
  mineToggle.addEventListener('click', () => {
    mine = !mine;
    mineToggle.setAttribute('aria-pressed', String(mine));
    mineToggle.classList.toggle('on', mine);
    void refresh();
  });
  pauseToggle.addEventListener('click', () => {
    paused = !paused;
    pauseToggle.setAttribute('aria-pressed', String(paused));
    pauseToggle.classList.toggle('on', paused);
    if (!paused) void refresh();
  });
  endButton.addEventListener('click', () => {
    const row = table.selected();
    if (row) confirmEnd(row, 'TERM');
  });

  // ---- ending one

  const send = async (row: ProcessRow, signal: string) => {
    try {
      await bridge.call('system.kill', { pid: row.pid, signal });
      status.show(
        signal === 'STOP' ? `Paused ${row.name}.` : signal === 'CONT' ? `Resumed ${row.name}.` : `Asked ${row.name} (${row.pid}) to close.`,
        'ok',
      );
      table.select(undefined);
      endButton.disabled = true;
      setTimeout(() => void refresh(), 300);
    } catch (e) {
      status.show(bridge.reason(e), 'error');
    }
  };

  /**
   * Ending a program you did not start, or forcing one, is worth a question.
   * Asking to close your own text editor is not.
   */
  const confirmEnd = (row: ProcessRow, signal: string) => {
    const hard = signal === 'KILL';
    if (!hard && row.own !== false) return void send(row, signal);
    const body = h(
      'div',
      { class: 'tk-confirm' },
      h('p', {}, hard ? `Force ${row.name} to stop immediately? Anything it has not saved will be lost.` : `End ${row.name}?`),
      h('p', { class: 'tk-confirm-meta' }, `PID ${row.pid}${row.user ? ` · running as ${row.user}` : ''}`),
      row.cmd ? h('code', { class: 'tk-confirm-cmd' }, row.cmd) : null,
    );
    const close = dialog(root, hard ? 'Force end' : 'End task', body, [
      h('button', { class: 'btn', onclick: () => close() }, 'Cancel'),
      h(
        'button',
        {
          class: 'btn danger',
          onclick: () => {
            close();
            void send(row, signal);
          },
        },
        hard ? 'Force end' : 'End task',
      ),
    ]);
  };

  // ---- what one process is doing

  const showDetail = async (row: ProcessRow) => {
    const facts = factGrid([['Loading', '…']], 'tk-facts-wide');
    const body = h('div', { class: 'tk-detail' }, facts.el);
    const close = dialog(root, row.name, body, [
      h('button', { class: 'btn', onclick: () => close() }, 'Close'),
      h(
        'button',
        {
          class: 'btn danger',
          onclick: () => {
            close();
            confirmEnd(row, 'TERM');
          },
        },
        'End task',
      ),
    ]);
    try {
      const d = await bridge.call<ProcessDetail>('system.process', { pid: row.pid });
      facts.set(
        [
          ['PID', String(d.pid)],
          ['Parent', d.ppid != null ? String(d.ppid) : '—'],
          ['User', row.user ?? '—'],
          ['State', stateName(d.state)],
          ['Threads', d.threads != null ? String(d.threads) : '—'],
          ['Open files', d.fds != null ? String(d.fds) : 'Not readable'],
          ['Memory', d.vmRss != null ? bytes(d.vmRss) : bytes(row.rss)],
          ['Peak memory', d.vmPeak != null ? bytes(d.vmPeak) : '—'],
          ['Swapped', d.vmSwap != null ? bytes(d.vmSwap) : '—'],
          ['Read', bytes(d.read)],
          ['Written', bytes(d.written)],
          ['Started', duration(Date.now() / 1000 - row.started) + ' ago'],
          ['Priority', `${row.prio} (nice ${row.nice})`],
          ['Switches', d.voluntary != null ? `${count(d.voluntary)} voluntary · ${count(d.involuntary ?? 0)} forced` : '—'],
          ['Windows program', d.wine ? 'Yes, under Wine' : 'No'],
          ['Control group', d.cgroup ?? '—'],
        ],
      );
      if (d.exe) body.append(h('div', { class: 'tk-subhead' }, 'Program'), h('code', { class: 'tk-code' }, d.exe));
      if (d.cmd) body.append(h('div', { class: 'tk-subhead' }, 'Command line'), h('code', { class: 'tk-code' }, d.cmd));
      if (d.cwd) body.append(h('div', { class: 'tk-subhead' }, 'Working directory'), h('code', { class: 'tk-code' }, d.cwd));
    } catch (e) {
      facts.set([['Unavailable', bridge.reason(e)]]);
    }
  };

  // ---- the right-click menu

  let openMenu: HTMLElement | undefined;
  const closeMenu = () => {
    openMenu?.remove();
    openMenu = undefined;
  };
  const menu = (row: ProcessRow, at: { x: number; y: number }) => {
    closeMenu();
    const item = (label: string, help: string, fn: () => void) =>
      h('button', { class: 'tk-menu-item', onclick: () => { closeMenu(); fn(); } }, h('b', {}, label), h('small', {}, help));
    const node = h(
      'div',
      { class: 'tk-menu', role: 'menu', style: { left: `${at.x}px`, top: `${at.y}px` } },
      item('Details', 'What this process is and what it has done', () => void showDetail(row)),
      ...SIGNALS.map((s) => item(s.label, s.help, () => confirmEnd(row, s.id))),
    );
    openMenu = node;
    root.appendChild(node);
    // Keep it on screen: a row near the bottom would otherwise open off it.
    const box = node.getBoundingClientRect();
    if (box.bottom > window.innerHeight) node.style.top = `${Math.max(8, at.y - box.height)}px`;
    if (box.right > window.innerWidth) node.style.left = `${Math.max(8, at.x - box.width)}px`;
    setTimeout(() => {
      window.addEventListener('pointerdown', closeMenu, { once: true, capture: true });
      window.addEventListener('keydown', (e) => e.key === 'Escape' && closeMenu(), { once: true });
    });
  };

  el.append(
    h(
      'p',
      { class: 'tk-note' },
      icon('info', 13),
      'Double-click a process for its details, or right-click it for everything that can be done to it. Only processes you own can be ended from here.',
    ),
  );

  const stop = every(el, TICK, () => void refresh());
  return () => {
    stop();
    closeMenu();
    status.clear();
  };
}

function stateName(state: string): string {
  const letter = state.trim().charAt(0);
  return (
    {
      R: 'Running',
      S: 'Sleeping',
      D: 'Waiting on disk',
      Z: 'Zombie',
      T: 'Paused',
      t: 'Being traced',
      I: 'Idle',
      X: 'Dead',
    }[letter] ?? state
  );
}
