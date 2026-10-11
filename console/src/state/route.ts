// The console's places, and their URLs. Pure: the History binding lives in store.ts.

export type Route =
  | { view: "overview" }
  | { view: "workspace"; name: string }
  | { view: "project"; name: string; settings?: true }
  | { view: "previews" }
  | { view: "settings" };

export function parseRoute(path: string): Route {
  const workspace = /^\/w\/([a-z0-9-]{1,63})\/?$/.exec(path);
  if (workspace?.[1]) return { view: "workspace", name: workspace[1] };
  const project = /^\/p\/([a-z0-9-]{1,63})(\/settings)?\/?$/.exec(path);
  if (project?.[1]) return project[2] ? { view: "project", name: project[1], settings: true } : { view: "project", name: project[1] };
  if (path === "/previews" || path === "/previews/") return { view: "previews" };
  if (path === "/settings" || path === "/settings/") return { view: "settings" };
  return { view: "overview" };
}

export function formatRoute(route: Route): string {
  switch (route.view) {
    case "overview":
      return "/";
    case "workspace":
      return `/w/${route.name}`;
    case "project":
      return route.settings ? `/p/${route.name}/settings` : `/p/${route.name}`;
    case "previews":
      return "/previews";
    case "settings":
      return "/settings";
  }
}
