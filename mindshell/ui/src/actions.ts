// Actions are the serialisable "what happens when you click this" used by
// menus and buttons; they only ever call the host, so a popup window can run
// an action that a panel window created.

import * as bridge from './bridge';
import { store } from './state';
import type { Action, Anchor } from './types';

export function openPopup(name: string, arg: Record<string, unknown> = {}, opts: { keyboard?: boolean; anchor?: Anchor } = {}): void {
  const anchor = opts.anchor ?? (arg.anchor as Anchor | undefined);
  bridge.send('popup.open', { name, keyboard: !!opts.keyboard, anchor, arg: { ...arg, anchor } });
}

export function togglePopup(name: string, arg: Record<string, unknown> = {}, opts: { keyboard?: boolean; anchor?: Anchor } = {}): void {
  const anchor = opts.anchor ?? (arg.anchor as Anchor | undefined);
  bridge.send('popup.toggle', { name, keyboard: !!opts.keyboard, anchor, arg: { ...arg, anchor } });
}

export function closePopup(name?: string): void {
  bridge.send('popup.close', name ? { name } : {});
}

export async function runAction(action: Action | undefined): Promise<void> {
  if (!action) return;
  if ('call' in action) {
    await bridge.call(action.call, action.params);
  } else if ('popup' in action) {
    openPopup(action.popup, (action.arg as Record<string, unknown>) ?? {}, { keyboard: action.keyboard });
  } else if ('editMode' in action) {
    await store.setEditMode(action.editMode);
  } else if ('exec' in action) {
    await bridge.call('shell.exec', { cmd: action.exec });
  } else if ('pin' in action) {
    const { panel, widget, app, pinned } = action.pin;
    await store.updateLayout((layout) => {
      const p = layout.panels.find((x) => x.id === panel);
      const w = p?.widgets.find((x) => x.id === widget);
      if (!w) return;
      const pins = Array.isArray(w.config.pins) ? (w.config.pins as string[]).filter((x) => x !== app) : [];
      if (pinned) pins.push(app);
      w.config.pins = pins;
    });
  }
}

export function launchApp(id: string): void {
  bridge.send('apps.launch', { id });
}

export function focusWindow(id: number): void {
  bridge.send('windows.focus', { id });
}
