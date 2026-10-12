// Where an address a program asked to open can go. Pure.
//
// A program in a workspace opens things through iglu's `xdg-open`, which
// asks the console. The person's own browser is where they're signed in,
// but it can't reach the workspace: an address on its loopback, or one of
// its files, only opens in the workspace's browser. The address comes from
// the workspace, so only web and file addresses are taken.

/** Where an address can open: in a tab of the person's own browser, or in
 * the workspace's browser. */
export type Place = "here" | "workspace";

export interface Offer {
  url: string;
  /** Where it can open, the person's browser first. */
  places: Place[];
}

/** Whether `host`, as `URL` gives it, is the machine itself. */
function loopback(host: string): boolean {
  return host === "localhost" || host.endsWith(".localhost") || /^127\.\d+\.\d+\.\d+$/.test(host) || host === "0.0.0.0" || host === "[::1]" || host === "[::]";
}

/** What `address` offers, or null for an address that isn't one or that
 * the console doesn't open. */
export function offer(address: string): Offer | null {
  let url: URL;
  try {
    url = new URL(address);
  } catch {
    return null;
  }
  switch (url.protocol) {
    case "http:":
    case "https:":
      return { url: url.href, places: loopback(url.hostname) ? ["workspace"] : ["here", "workspace"] };
    case "file:":
      return { url: url.href, places: ["workspace"] };
    default:
      return null;
  }
}
