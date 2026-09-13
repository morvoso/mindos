// One place that knows how the Task Manager is opened, because three
// different cards offer to open it.
//
// It is a window of its own rather than a page embedded in the desktop. The
// desktop is a layer-shell surface behind every other window: a task manager
// drawn there would sit underneath the very program someone opened it to end.

import * as bridge from './bridge';

export function openTaskManager(page?: string): void {
  bridge.send('shell.openApp', { name: 'tasks', page });
}
