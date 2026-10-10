// Changed by iglu, not upstream (see ../UPSTREAM):
// - The host is always focusable by pointer (tabindex -1), never a tab stop.
// - The Escape-then-Tab hint is left out when that gesture is off.

const LABEL_ATTRIBUTES = [
  "aria-label",
  "aria-labelledby",
  "aria-describedby",
  "aria-description",
] as const;

const KEYBOARD_EXIT_HINT =
  "Press Escape, then Tab to move focus out of the terminal, or Shift+Tab to move focus backward.";
let nextHintId = 0;

/** Keep host-provided names and tab order on the element receiving input. */
export class InputAccessibility {
  private observer: MutationObserver;
  private tabIndex: string | null;
  private defaultRole: boolean;
  private destroyed = false;
  private hint: HTMLSpanElement;

  constructor(
    private host: HTMLElement,
    private input: HTMLTextAreaElement,
    /** Whether Escape then Tab leaves the terminal, and so is announced. */
    private exitHint = true,
  ) {
    this.hint = host.ownerDocument.createElement("span");
    do {
      this.hint.id = `wterm-input-help-${++nextHintId}`;
    } while (
      (host.getRootNode() as ParentNode).querySelector(`#${this.hint.id}`)
    );
    this.hint.hidden = true;
    this.hint.textContent = KEYBOARD_EXIT_HINT;
    host.append(this.hint);
    this.tabIndex = host.getAttribute("tabindex");
    this.defaultRole = !host.hasAttribute("role");
    if (this.defaultRole) host.setAttribute("role", "group");
    this.observer = new MutationObserver((records) => this.sync(records));
    this.sync();
  }

  private sync(records: MutationRecord[] = []): void {
    if (records.some((record) => record.attributeName === "tabindex"))
      this.tabIndex = this.host.getAttribute("tabindex");
    for (const attribute of LABEL_ATTRIBUTES) {
      const value = this.host.getAttribute(attribute);
      if (value?.trim()) this.input.setAttribute(attribute, value);
      else this.input.removeAttribute(attribute);
    }
    if (!this.input.hasAttribute("aria-label"))
      this.input.setAttribute("aria-label", "Terminal");
    // Described-by references take precedence over aria-description. Append
    // instructions to both paths, preserving the host's own description.
    const describedBy = this.input.getAttribute("aria-describedby");
    if (describedBy && this.exitHint)
      this.input.setAttribute(
        "aria-describedby",
        `${describedBy} ${this.hint.id}`,
      );
    const description = [
      this.input.getAttribute("aria-description"),
      this.exitHint ? KEYBOARD_EXIT_HINT : null,
    ]
      .filter(Boolean)
      .join(" ");
    if (description) this.input.setAttribute("aria-description", description);
    else this.input.removeAttribute("aria-description");
    this.input.setAttribute("tabindex", this.tabIndex ?? "0");

    // The host and textarea must not become two sequential tab stops. The
    // host still takes focus from the pointer, while text in it is selected,
    // so it's always -1: the tab stop is the textarea's. Do not observe our
    // own normalization as a new host request; framework updates (including
    // a repeated -1) still reach the observer.
    this.observer.disconnect();
    this.host.setAttribute("tabindex", "-1");
    this.observer.observe(this.host, {
      attributes: true,
      attributeFilter: [...LABEL_ATTRIBUTES, "tabindex"],
    });
  }

  destroy(): void {
    if (this.destroyed) return;
    this.destroyed = true;
    const pending = this.observer.takeRecords();
    if (pending.some((record) => record.attributeName === "tabindex"))
      this.tabIndex = this.host.getAttribute("tabindex");
    this.observer.disconnect();
    this.hint.remove();
    if (this.tabIndex === null) this.host.removeAttribute("tabindex");
    else this.host.setAttribute("tabindex", this.tabIndex);
    if (this.defaultRole && this.host.getAttribute("role") === "group")
      this.host.removeAttribute("role");
  }
}
