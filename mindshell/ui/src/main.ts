import './widgets';
import { renderApp } from './apps';
import * as bridge from './bridge';
import { renderDesktop } from './desktop';
import { renderGreeter } from './greeter';
import { installMock } from './mock';
import { renderPanel } from './panel';
import { renderPopupWindow } from './popups';
import { renderPreview } from './preview';
import { store } from './state';

async function main(): Promise<void> {
  if (!bridge.hasHost()) installMock();
  const info = bridge.windowInfo();
  document.documentElement.dataset.kind = info.kind;
  await store.init();
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
    default:
      renderPreview(root);
  }
  bridge.send('shell.ready', { kind: info.kind, id: info.id, popup: info.popup });
}

main().catch((e) => {
  console.error(e);
  document.body.textContent = `mindshell failed to start: ${e}`;
});
