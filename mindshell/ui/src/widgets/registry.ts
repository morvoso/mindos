// The widget registry. A widget type is a small module that registers a
// definition; panels and the desktop instantiate widgets from layout entries.

import type { Anchor, Config, Container, Edge } from '../types';
import type { Store } from '../state';

export interface SettingSpec {
  label: string;
  type: 'boolean' | 'number' | 'string' | 'text' | 'enum' | 'list';
  min?: number;
  max?: number;
  step?: number;
  /** Shown after a number (px, %, s). */
  unit?: string;
  /** `number` with a min and max gets a slider unless this is false. */
  slider?: boolean;
  options?: { value: string | number | boolean; label: string }[];
  /** `enum`: a row of buttons instead of a drop-down (best for 2–4 short options). */
  segmented?: boolean;
  help?: string;
  placeholder?: string;
  /** Hide the row unless this says otherwise (the form re-checks after every change). */
  when?: (config: Config) => boolean;
}

export interface WidgetCtx {
  id: string;
  type: string;
  config: Config;
  container: Container;
  output: string;
  panel?: { id: string; edge: Edge; size: number; vertical: boolean };
  store: Store;
  /** Origin of this window in output coordinates. */
  origin(): { x: number; y: number };
  /** Output-space rectangle of an element, tagged with the panel edge. */
  anchorOf(el: Element): Anchor;
  openPopup(name: string, arg?: Record<string, unknown>, opts?: { keyboard?: boolean; anchor?: Anchor }): void;
  togglePopup(name: string, arg?: Record<string, unknown>, opts?: { keyboard?: boolean; anchor?: Anchor }): void;
  setConfig(patch: Config): void;
  editMode(): boolean;
}

export interface WidgetInstance {
  el: HTMLElement;
  update?(config: Config): void;
  destroy?(): void;
}

export interface WidgetDef {
  type: string;
  name: string;
  description: string;
  icon: string;
  containers: Container[];
  defaults: Config;
  settings?: Record<string, SettingSpec>;
  /** Default size when placed on the desktop. */
  defaultSize?: { w: number; h: number };
  create(ctx: WidgetCtx): WidgetInstance;
}

const defs = new Map<string, WidgetDef>();

export function registerWidget(def: WidgetDef): void {
  defs.set(def.type, def);
}

export function getWidget(type: string): WidgetDef | undefined {
  return defs.get(type);
}

export function allWidgets(container?: Container): WidgetDef[] {
  const list = [...defs.values()];
  return container ? list.filter((d) => d.containers.includes(container)) : list;
}

export function mergedConfig(def: WidgetDef | undefined, config: Config): Config {
  return { ...(def?.defaults ?? {}), ...(config ?? {}) };
}
