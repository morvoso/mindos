// What a screensaver is, from the outside.

export interface Saver {
  /** Stored in the preferences (`idle.saver`). */
  id: string;
  name: string;
  description: string;
  /** Start drawing on `canvas`; the returned function stops it and lets go
   *  of every timer and observer it took. */
  start(canvas: HTMLCanvasElement): () => void;
}
