import './widgets';
import { initAppearance } from './appearance';
import { installController } from './controller';
import { renderApp } from './apps';
import * as bridge from './bridge';
import { renderDesktop } from './desktop';
import { renderGreeter } from './greeter';
import { renderLock } from './lock';
import { installMock } from './mock';
import { renderPanel } from './panel';
import { renderPopupWindow } from './popups';
import { renderPreview } from './preview';
import { setQuiet } from './quiet';
import { store } from './state';
import { renderToasts } from './toast';

async function main(): Promise<void> {
  if (!bridge.hasHost()) installMock();
  const info = bridge.windowInfo();
  document.documentElement.dataset.kind = info.kind;
  await store.init();
  initAppearance();
  // Quiet while a game runs: no animations, samplers slowed or stopped.
  setQuiet(!!store.state.game);
  store.on('game', () => setQuiet(!!store.state.game));
  const root = document.body;
  switch (info.kind) {
    case 'desktop':
      renderDesktop(root, info.output);
      break;
    case 'panel':
      renderPanel(root, info.id, info.output);
      break;
    case 'popup':
      renderPopupWindow(root, info.popup ?? '', info.arg, info.output, () => bridge.send('popup.close', { name: info.popup }));
      break;
    case 'app':
      renderApp(root, info.id, info.arg);
      break;
    case 'greeter':
      renderGreeter(root, info.arg);
      break;
    case 'lock':
      renderLock(root, info.arg);
      break;
    case 'toast':
      renderToasts(root, info.output);
      break;
    default:
      renderPreview(root);
  }
  bridge.send('shell.ready', { kind: info.kind, id: info.id, popup: info.popup });
  if (!['toast', 'panel'].includes(info.kind)) installController();
}

main().catch((e) => {
  console.error(e);
  document.body.textContent = `mindshell failed to start: ${e}`;
});
