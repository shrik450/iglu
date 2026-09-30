// A tiny element builder. Text is always set as text, never parsed as HTML.

type Child = Node | string | null | undefined | false;
type Props = Record<string, string | boolean | ((event: Event) => void) | undefined>;

export function h<K extends keyof HTMLElementTagNameMap>(tag: K, props: Props = {}, ...children: Child[]): HTMLElementTagNameMap[K] {
  const element = document.createElement(tag);
  for (const [key, value] of Object.entries(props)) {
    if (value === undefined || value === false) continue;
    if (typeof value === "function") {
      element.addEventListener(key.replace(/^on/, ""), value);
    } else if (value === true) {
      element.setAttribute(key, "");
    } else if (key === "class") {
      element.className = value;
    } else {
      element.setAttribute(key, value);
    }
  }
  for (const child of children) {
    if (child === null || child === undefined || child === false) continue;
    element.append(typeof child === "string" ? document.createTextNode(child) : child);
  }
  return element;
}

export function replace(parent: Element, ...children: Child[]): void {
  parent.replaceChildren(...children.filter((c): c is Node | string => c !== null && c !== undefined && c !== false));
}
