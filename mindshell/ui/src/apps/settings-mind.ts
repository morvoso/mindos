// Settings › Mind: how answers are shown, which model runs, and downloads.

import * as bridge from '../bridge';
import { h } from '../dom';
import { icon } from '../icons';
import { store } from '../state';
import type { CatalogEntry, ModelsInfo } from '../types';
import { card, fmtBytes, notice, pageHeader, pill, progress, row, toggle } from './shared';

const request = (req: Record<string, unknown>) => bridge.call<ModelsInfo>('mind.request', { request: req });

export function mindPage(el: HTMLElement): () => void {
  const note = notice();
  let info: ModelsInfo | undefined;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let busyUntil = 0;

  const fail = (e: unknown) => note.show(`Mind: ${e instanceof Error ? e.message : String(e)}`, 'error');

  // ----- answers ------------------------------------------------------------
  const toolsToggle = toggle(!!store.prefs.mind_show_tools, (v) => void store.setPrefs({ mind_show_tools: v }));
  const thinkToggle = toggle(false, (v) => {
    request({ type: 'set_thinking', enabled: v })
      .then((m) => {
        info = m;
        render();
      })
      .catch(fail);
  });
  const answers = card(
    'Answers',
    row('Show what Mind is doing', 'The commands Mind runs on the way to an answer (the yellow lines) appear above it. Off, you only see the answer.', toolsToggle),
    row('Think before answering', 'Mind reasons through the question first. Slower, and better on hard questions. Qwen3.5 supports it; leave it off for quick answers.', thinkToggle),
  );

  // ----- model ----------------------------------------------------------------
  const modelBody = h('div', { class: 'stack' });
  const modelCard = card('Model', modelBody);
  const catalogBody = h('div', { class: 'catalog-grid' });
  const catalogCard = card('Get a model', h('p', { class: 'card-help' }, 'Models are downloaded from Hugging Face into your models folder with their licence. Bigger models answer better and need more video memory.'), catalogBody);
  const pathIn = h('input', { type: 'text', class: 'grow', placeholder: '/path/to/model.gguf', spellcheck: 'false' }) as HTMLInputElement;
  const useBtn = h('button', { class: 'btn', onclick: () => {
    const p = pathIn.value.trim();
    if (!p) return;
    setModel(p);
  } }, icon('check', 14), 'Use this file');
  const ownHelp = h('div', { class: 'row-help' });
  const ownCard = card('Use your own model', h('div', { class: 'inline-form' }, pathIn, useBtn), ownHelp);

  el.append(pageHeader('Mind', 'The assistant behind Super+Space. Ask it to open apps, install things, change settings or explain what is going on.'), note.el, answers, modelCard, catalogCard, ownCard);

  const setModel = (path: string) => {
    busyUntil = Date.now() + 3000;
    request({ type: 'set_model', path })
      .then((m) => {
        info = m;
        note.show(path === 'auto' || path === '' ? 'Mind will pick the largest model that fits.' : `Switching to ${path.split('/').pop()}…`, 'ok');
        render();
        schedule();
      })
      .catch(fail);
  };

  const download = (c: CatalogEntry) => {
    request({ type: 'download_model', url: c.url, file: c.file, size: c.size, use_after: true })
      .then((m) => {
        info = m;
        note.show(`Downloading ${c.name}…`, 'ok');
        render();
        schedule();
      })
      .catch(fail);
  };

  const cancel = () => {
    request({ type: 'cancel_download' }).catch(() => undefined);
    setTimeout(refresh, 300);
  };

  const gpuGb = () => (info?.gpu_memory ? info.gpu_memory / 1073741824 : undefined);

  const renderModel = () => {
    modelBody.replaceChildren();
    if (!info) {
      modelBody.appendChild(h('div', { class: 'row-help' }, 'Connecting to Mind…'));
      return;
    }
    const status = info.external ? 'External server' : info.ready ? 'Ready' : info.current ? 'Loading…' : 'No model loaded';
    const badges = h('span', { class: 'pills' });
    if (info.auto && !info.external) badges.appendChild(pill('Automatic', 'accent'));
    if (info.thinking) badges.appendChild(pill('Thinking on', 'mind'));
    const g = gpuGb();
    if (g) badges.appendChild(pill(`${Math.round(g)} GB GPU`));
    modelBody.appendChild(
      h('div', { class: 'model-now' }, h('span', { class: 'model-ic', dataset: { state: info.external ? 'ready' : info.ready ? 'ready' : info.current ? 'loading' : 'off' } }, icon('mind', 22)), h('div', { class: 'model-text' }, h('div', { class: 'model-name' }, info.model || 'None'), h('div', { class: 'row-help' }, status)), badges),
    );
    if (info.external) {
      modelBody.appendChild(h('div', { class: 'row-help' }, 'Mind is talking to an external server; models are managed there.'));
      return;
    }
    const list = h('div', { class: 'list' });
    list.appendChild(
      row('Automatic', 'Pick the largest installed model that fits the GPU.', info.auto ? pill('On', 'ok') : h('button', { class: 'btn small', onclick: () => setModel('auto') }, 'Use')),
    );
    for (const m of info.models) {
      const tooBig = false;
      list.appendChild(
        row(m.file, `${fmtBytes(m.size)}${tooBig ? ' · needs more video memory' : ''}`, m.active ? pill('In use', 'ok') : h('button', { class: 'btn small', onclick: () => setModel(m.path) }, 'Use')),
      );
    }
    if (!info.models.length) list.appendChild(h('div', { class: 'row-help' }, `No models in ${info.models_dir} yet. Download one below.`));
    modelBody.appendChild(list);
  };

  const renderCatalog = () => {
    catalogBody.replaceChildren();
    if (!info) return;
    const dl = info.download && !info.download.done ? info.download : undefined;
    const failed = info.download && info.download.done && info.download.error ? info.download : undefined;
    const g = gpuGb();
    for (const c of info.catalog) {
      const fits = g === undefined || c.min_vram_gb <= g;
      const inUse = info.models.find((m) => m.file === c.file)?.active;
      const busy = dl?.file === c.file;
      const badges = h('div', { class: 'pills' });
      if (c.recommended) badges.appendChild(pill('Recommended', 'accent'));
      if (c.installed) badges.appendChild(pill('Installed', 'ok'));
      if (inUse) badges.appendChild(pill('In use', 'ok'));
      if (!fits) badges.appendChild(pill(`Needs ${c.min_vram_gb} GB`, 'warn'));
      let action: HTMLElement;
      if (busy && dl) {
        const frac = dl.total ? dl.received / dl.total : 0;
        action = h('div', { class: 'dl' }, progress(frac, 'accent'), h('div', { class: 'dl-text mono' }, `${fmtBytes(dl.received)} / ${fmtBytes(dl.total || c.size)} · ${Math.round(frac * 100)}%`), h('button', { class: 'btn small danger', onclick: cancel }, 'Cancel'));
      } else if (c.installed) {
        action = inUse ? h('span') : h('button', { class: 'btn small', onclick: () => setModel(`${info!.models_dir}/${c.file}`) }, 'Use');
      } else {
        action = h('button', { class: 'btn small accent', disabled: !!dl, onclick: () => download(c) }, icon('download', 14), 'Download');
      }
      const err = failed?.file === c.file ? h('div', { class: 'row-help danger' }, `Download failed: ${failed.error}`) : null;
      catalogBody.appendChild(
        h(
          'div',
          { class: `cat-model${c.recommended ? ' rec' : ''}${!fits ? ' dim' : ''}` },
          h('div', { class: 'cat-model-head' }, h('div', { class: 'cat-model-name' }, c.name), badges),
          h('div', { class: 'cat-model-meta mono' }, `${c.params} · ${fmtBytes(c.size)} · ${c.license}`),
          h('div', { class: 'cat-model-desc' }, c.description),
          err,
          h('div', { class: 'cat-model-foot' }, h('button', { class: 'linkish', onclick: () => bridge.send('shell.exec', { cmd: `xdg-open ${c.license_url}` }) }, icon('external', 12), 'Licence'), h('span', { class: 'strip-gap' }), action),
        ),
      );
    }
    if (!info.catalog.length) catalogBody.appendChild(h('div', { class: 'row-help' }, 'No catalog is installed (see /etc/mindos/model-catalog.json).'));
  };

  const render = () => {
    const think = thinkToggle.querySelector('input') as HTMLInputElement;
    if (info && document.activeElement !== think) think.checked = !!info.thinking;
    think.disabled = !info;
    renderModel();
    renderCatalog();
    ownHelp.textContent = info ? `Any GGUF works. Files in ${info.models_dir} show up in the list above.` : 'Any GGUF file works.';
  };

  const refresh = () => {
    request({ type: 'models' })
      .then((m) => {
        info = m;
        note.clear();
        render();
        schedule();
      })
      .catch((e) => {
        info = undefined;
        render();
        fail(e);
        schedule();
      });
  };
  const schedule = () => {
    if (timer) clearTimeout(timer);
    if (!el.isConnected) return;
    const busy = !!info && ((!!info.download && !info.download.done) || (!info.ready && !!info.current) || Date.now() < busyUntil);
    timer = setTimeout(refresh, busy ? 1000 : 8000);
  };

  render();
  refresh();
  void store.fetchPrefs().then(() => {
    const t = toolsToggle.querySelector('input') as HTMLInputElement;
    t.checked = !!store.prefs.mind_show_tools;
  });
  store.bind(el, 'prefs', () => {
    const t = toolsToggle.querySelector('input') as HTMLInputElement;
    if (document.activeElement !== t) t.checked = !!store.prefs.mind_show_tools;
  });
  return () => {
    if (timer) clearTimeout(timer);
  };
}
