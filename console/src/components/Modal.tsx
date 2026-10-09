// A dialog over the page, as the browser's own modal <dialog>: Tab stays
// inside it, the page behind can't take focus or clicks (so a terminal can't
// pull focus back), Escape closes it, and focus returns to where it was when
// it closes. It takes focus on the element marked `autofocus`, else its first.

import type { ComponentChildren } from "preact";
import { useLayoutEffect, useRef } from "preact/hooks";

export function Modal(props: { label?: string; labelledby?: string; onClose: () => void; children: ComponentChildren }) {
  const ref = useRef<HTMLDialogElement>(null);
  useLayoutEffect(() => {
    const dialog = ref.current;
    dialog?.showModal();
    return () => dialog?.close();
  }, []);
  return (
    <dialog
      ref={ref}
      class="overlay"
      aria-label={props.label}
      aria-labelledby={props.labelledby}
      onCancel={(e) => {
        e.preventDefault();
        props.onClose();
      }}
      // The dialog fills the screen around its box: a click on it, not the box, is a click outside.
      onClick={(e) => e.target === e.currentTarget && props.onClose()}
    >
      {props.children}
    </dialog>
  );
}
