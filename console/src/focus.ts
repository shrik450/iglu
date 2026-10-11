// Who has the keyboard, for a column that opens to know whether to take it.

/** Whether the keyboard is free for an opening column to take: it isn't
 * while someone types into a field, such as a column's name, or a dialog
 * has it. A link or button just used, such as a workspace's in the sidebar,
 * or another column, doesn't keep it. */
export function keyboardFree(active: Element | null = document.activeElement): boolean {
  if (!active || active === document.body) return true;
  if (active.closest('[role="dialog"], [role="alertdialog"], dialog, .info')) return false;
  if (active.closest(".term-host, .browser-screen")) return true;
  return !active.matches('input, textarea, select, [contenteditable]:not([contenteditable="false"])');
}
