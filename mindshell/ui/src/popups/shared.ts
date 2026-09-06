import type { Anchor } from '../types';
import type { Store } from '../state';

export interface PopupCtx {
  name: string;
  arg: Record<string, unknown>;
  output: string;
  anchor?: Anchor;
  store: Store;
  close(): void;
  /** Ask the router to re-place the popup after its size changed. */
  relayout(): void;
}

export interface PopupContent {
  el: HTMLElement;
  /** Fixed width in px; omit for content-sized. */
  w?: number;
  /** Fixed height in px; omit for content-sized. */
  h?: number;
  focus?(): void;
}

export type PopupFactory = (ctx: PopupCtx) => PopupContent;

/** Two-step confirmation for destructive buttons: first click arms, second fires. */
export function armable(btn: HTMLElement, label: HTMLElement, text: string, confirm: string, fire: () => void, ms = 3000): void {
  let armed = false;
  let t: ReturnType<typeof setTimeout> | undefined;
  btn.addEventListener('click', () => {
    if (!armed) {
      armed = true;
      btn.classList.add('armed');
      label.textContent = confirm;
      t = setTimeout(() => {
        armed = false;
        btn.classList.remove('armed');
        label.textContent = text;
      }, ms);
      return;
    }
    if (t) clearTimeout(t);
    fire();
  });
}
