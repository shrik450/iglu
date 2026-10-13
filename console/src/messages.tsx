// Every word the console shows, in one place, to read and change together.
//
// Each message is a whole sentence, label or phrase, made from typed values
// and never pieced together where it's shown, so a translation can reorder,
// inflect or pluralize it as its language needs. Plurals, lists and numbers
// go through Intl, not English rules. Another language would be another
// object of the type `Messages`, so one that misses a message doesn't build.
// What isn't the console's to say stays as it came, and isn't translated:
// names, paths, addresses, keys, and what a host or program said itself.

import type { ComponentChildren } from "preact";

import type { Refusal, SecretPath } from "./api/client.ts";
import type { Rules } from "./components/forms.tsx";
import type { Activity } from "./generated/Activity.ts";
import type { ChangeProject } from "./generated/ChangeProject.ts";
import type { Closed } from "./generated/Closed.ts";
import type { ColumnKind } from "./generated/ColumnKind.ts";
import type { CreateEnvironment } from "./generated/CreateEnvironment.ts";
import type { CreateProject } from "./generated/CreateProject.ts";
import type { CreateWorkspace } from "./generated/CreateWorkspace.ts";
import type { DesiredState } from "./generated/DesiredState.ts";
import type { Effect } from "./generated/Effect.ts";
import type { Failure } from "./generated/Failure.ts";
import type { IdleRule } from "./generated/IdleRule.ts";
import type { LabelColumn } from "./generated/LabelColumn.ts";
import type { LabelWorkspace } from "./generated/LabelWorkspace.ts";
import type { Look } from "./generated/Look.ts";
import type { OptionAsMeta } from "./generated/OptionAsMeta.ts";
import type { Permission } from "./generated/Permission.ts";
import type { Phase } from "./generated/Phase.ts";
import type { PublishPort } from "./generated/PublishPort.ts";
import type { PutSecret } from "./generated/PutSecret.ts";
import type { Reason } from "./generated/Reason.ts";
import type { RenameWorkspace } from "./generated/RenameWorkspace.ts";
import type { SecretTarget } from "./generated/SecretTarget.ts";
import type { Unsaved } from "./generated/Unsaved.ts";
import type { Action, Group } from "./state/keys.ts";
import type { Situation } from "./state/situation.ts";
import type { ThemeProblem } from "./state/themes.ts";
import { unreachable } from "./state/unsaved.ts";

/** The language these messages are in, for Intl's plurals, lists and numbers. */
const LOCALE = "en";

const plurals = new Intl.PluralRules(LOCALE);
const lists = new Intl.ListFormat(LOCALE, { type: "conjunction" });
const numbers = new Intl.NumberFormat(LOCALE);
const tenths = new Intl.NumberFormat(LOCALE, { minimumFractionDigits: 1, maximumFractionDigits: 1 });

/** The form of a phrase for `count`, by the language's plural rules. */
function plural(count: number, forms: { one: string; other: string }): string {
  return plurals.select(count) === "one" ? forms.one : forms.other;
}

/** Bytes in binary gigabytes, as the core says memory: "1.5 GiB". */
function gibibytes(bytes: number): string {
  return `${tenths.format(bytes / 1024 ** 3)} GiB`;
}

/** Why iglu refused a request, as the console says it. */
function reason(reason: Reason): string {
  switch (reason.kind) {
    case "Invalid":
      return "That isn't something iglu can take.";
    case "NotJson":
    case "JsonExpected":
    case "FormExpected":
    case "MethodNotAllowed":
    case "IdempotencyKeyInvalid":
      return "iglu couldn't read what this page sent. Reload the page.";
    case "Unreadable":
      return "That didn't reach iglu whole. Try again.";
    case "NotWebSocket":
      return "That needs a live connection, which this page didn't open. Reload the page.";
    case "ProjectUnnamed":
      return "Give the project a name or a repository.";
    case "NothingToType":
      return "Say what to type, or to press Enter.";
    case "NoWorkspaceToGrant":
      return "There's no such workspace to give that access to.";
    case "BrowserPort":
      return `Port ${reason.port} can't be published: it's the workspace browser's debugging port or its display, and a preview of either would let anyone with the link drive the browser.`;
    case "SignInIncomplete":
      return "The sign-in came back without what iglu needs. Try again.";
    case "OtherBrowser":
      return "Finish signing in in the browser that started it.";
    case "SignInExpired":
      return "This sign-in expired. Try again.";
    case "CliSignInMalformed":
      return "That request to sign in the iglu CLI isn't one iglu reads.";
    case "BadReturnAddress":
      return "That sign-in was asked to return somewhere other than a preview.";
    case "Unauthorized":
      return "Your session ended. Signing in again…";
    case "AccountDisabled":
      return "This account is disabled.";
    case "NotGranted":
      return "This workspace hasn't been given that.";
    case "PersonOnly":
      return "Only a person can do that, not a workspace.";
    case "NotFromWorkspace":
      return "A workspace can't do that from inside; its owner can.";
    case "CrossOrigin":
      return "iglu refused a request from another site.";
    case "SignInFailed":
      return "Signing in failed.";
    case "ProviderRefused":
      return `The identity provider refused, saying ${reason.error}.`;
    case "AccountNotAllowed":
      return "This account isn't allowed to use iglu.";
    case "NotFound":
      return "That isn't there any more.";
    case "NotRunning":
      return "The workspace isn't running.";
    case "IdempotencyKeyReused":
      return "iglu took that for a repeat of another request. Try again.";
    case "NoSuchProject":
      return "That project isn't there any more.";
    case "NoEnvironment":
      return `There's no environment called ${reason.name}.`;
    case "NotBuilt":
      return `${reason.environment} hasn't finished a build yet. Wait for it, or rebuild it in Settings.`;
    case "NoRepository":
      return "The project has no repository, so its workspaces have no branch.";
    case "WorkspaceNameTaken":
      return "Another workspace has that name.";
    case "ProjectNameTaken":
      return "Another project has that name.";
    case "EnvironmentNameTaken":
      return "Another environment has that name.";
    case "BeingDeleted":
      return "The workspace is being deleted.";
    case "FreezeNeedsRunning":
      return "Only a running workspace can be frozen.";
    case "NoBuildToUpdateTo":
      return "Its environment has no build ready to update to.";
    case "BuildForAnotherAccount":
      return "The newest build runs as another account, whose files would be someone else's. Make a new workspace to use it.";
    case "NoBrowserInImage":
      return "This workspace's environment has no browser. Turn on iglu.browser.enable, rebuild the environment, then update the workspace.";
    case "BrowserColumnExists":
      return `This workspace has a browser already, in ${reason.column}.`;
    case "ColumnNameTaken":
      return `There's a column called ${reason.name} already.`;
    case "LayoutMismatch":
      return "The columns changed meanwhile. Try again.";
    case "SecretTargetTaken":
      return `${reason.by} already goes there.`;
    case "ProjectChanged":
      return "The project changed since you loaded it.";
    case "BuiltinProject":
      return "The built-in project can't be removed.";
    case "ProjectInUse":
      return "The project still has workspaces.";
    case "TooLarge":
      return "That's too large for iglu to take.";
    case "ProviderUnreachable":
      return "The identity provider isn't answering. Try again shortly.";
    case "NoHost":
      return "iglu has no host to run workspaces on.";
    case "HostUnknown":
      return "This workspace's host isn't in iglu's configuration.";
    case "HostFailed":
      return `The workspace's host couldn't do that. It said: ${reason.detail}`;
    case "Internal":
      return "Something went wrong in iglu. Its log says more.";
    default:
      return unreachable(reason);
  }
}

/** Why a step on a workspace failed, as a sentence. */
function failure(failure: Failure): string {
  switch (failure.kind) {
    case "Host":
      return `Its host said: ${failure.detail}`;
    case "SecretsClash": {
      const clash = failure.clash;
      switch (clash.kind) {
        case "DuplicateEnv":
          return `Two of your secrets set the environment variable ${clash.name}. Change one in Settings.`;
        case "DuplicateFile":
          return `Two of your secrets write ~/${clash.path}. Change one in Settings.`;
        case "DuplicateGitHost":
          return `Two of your secrets are Git credentials for ${clash.host}. Change one in Settings.`;
        default:
          return unreachable(clash);
      }
    }
    case "Internal":
      return "Something went wrong in iglu. Its log says more.";
    default:
      return unreachable(failure);
  }
}

/** A step that failed, in a workspace's history. */
function couldnt(step: Effect): string {
  switch (step) {
    case "Create":
      return "Couldn't create it";
    case "Start":
      return "Couldn't start it";
    case "DeliverSecrets":
      return "Couldn't deliver its secrets";
    case "Provision":
      return "Couldn't set it up";
    case "OpenColumns":
      return "Couldn't open its columns";
    case "Freeze":
      return "Couldn't freeze it";
    case "Thaw":
      return "Couldn't thaw it";
    case "Stop":
      return "Couldn't stop it";
    case "Delete":
      return "Couldn't remove it from its host";
    case "Upgrade":
      return "Couldn't upgrade it to the newest build";
    default:
      return unreachable(step);
  }
}

/** What a workspace was asked to be, in its history. */
function asked(state: DesiredState): string {
  switch (state) {
    case "running":
      return "Asked to run";
    case "frozen":
      return "Asked to freeze";
    case "stopped":
      return "Asked to stop";
    case "deleted":
      return "Asked to be deleted";
    default:
      return unreachable(state);
  }
}

/** A history entry: what happened, and what it happened to or with. */
function happened(what: Activity): { what: string; detail: string | null } {
  const only = (text: string) => ({ what: text, detail: null });
  switch (what.kind) {
    case "Requested":
      return { what: "Asked for", detail: what.checkout ? `${what.checkout.branch} on ${what.checkout.repo}` : `in ${what.project}` };
    case "StateSet":
      return only(asked(what.state));
    case "Created":
      return only("Created");
    case "Started":
      return only("Started");
    case "SecretsDelivered":
      return only("Secrets delivered");
    case "Provisioned":
      return only("Set up");
    case "ColumnsOpened":
      return only("Columns opened");
    case "Frozen":
      return only("Frozen");
    case "Thawed":
      return only("Thawed");
    case "Stopped":
      return only("Stopped");
    case "Removed":
      return only("Removed from its host");
    case "Deleted":
      return only("Deleted");
    case "Error":
      return { what: couldnt(what.step), detail: failure(what.failure) };
    case "Renamed":
      return { what: "Renamed", detail: `${what.from} to ${what.to}` };
    case "Relabeled":
      return what.to === null ? only("Called by its name again") : { what: "Called", detail: what.to };
    case "Typed":
      return { what: "Typed", detail: `into ${what.column}` };
    case "ThawedOnOpen":
      return only("Thawed when opened");
    case "IdleFroze":
      return only("Frozen while idle");
    case "IdleStopped":
      return only("Stopped while idle");
    case "SecretUpdated":
      return { what: "Secret updated", detail: what.secret };
    case "Published":
      return { what: "Published", detail: `port ${what.port} as ${what.route}` };
    case "Unpublished":
      return { what: "Unpublished", detail: `port ${what.port} as ${what.route}` };
    case "FileKept":
      return { what: "Kept a file", detail: what.path };
    case "Adopted":
      switch (what.reason) {
        case "FrozenStateLost":
          return { what: "Changed by its host", detail: "The host restarted, so the frozen workspace is now stopped." };
        default:
          return unreachable(what.reason);
      }
    case "AccessChanged":
      return only("Access changed");
    case "ColumnAdded":
      return { what: "Added a column", detail: what.column };
    case "ColumnRestarted":
      return { what: "Restarted a column", detail: what.column };
    case "ColumnClosed":
      return { what: "Closed a column", detail: what.column };
    case "UpdateAsked":
      return { what: "Asked to update", detail: `to ${what.environment}'s newest build` };
    case "Updated":
      return only("Updated to the newest build");
    default:
      return unreachable(what);
  }
}

/** A trouble's title when a step keeps failing, by the phase it's stuck in. */
function failing(phase: Phase): string {
  switch (phase) {
    case "creating":
      return "Can't create it yet";
    case "starting":
      return "Can't start it yet";
    case "freezing":
      return "Can't freeze it yet";
    case "stopping":
      return "Can't stop it yet";
    case "deleting":
    case "deleted":
      return "Can't delete it yet";
    case "running":
    case "frozen":
    case "stopped":
      return "Something keeps failing";
    default:
      return unreachable(phase);
  }
}

/** What a workspace in trouble shows: a title, and what to know about it. */
function trouble(now: Extract<Situation, { kind: "stuck" | "broken" | "held" }>): { title: string; detail: string } {
  switch (now.kind) {
    case "stuck":
      return { title: failing(now.phase), detail: `${failure(now.failure)} iglu keeps trying.` };
    case "broken":
      return { title: "It's broken", detail: "The host can't run it any more. Stop it to try again, or delete it." };
    case "held":
      switch (now.condition.kind) {
        case "HostOffline":
          return { title: "The host isn't answering", detail: "It carries on when the host is back." };
        case "Capacity":
          return {
            title: "Waiting for room",
            detail: `It needs ${gibibytes(now.condition.needed)} and the host has ${gibibytes(now.condition.available)} free. It starts when another workspace frees some.`,
          };
        default:
          return unreachable(now.condition);
      }
    default:
      return unreachable(now);
  }
}

/** What a workspace that isn't up or in trouble shows instead of its columns. */
function resting(now: Extract<Situation, { kind: "building" | "frozen" | "stopped" | "leaving" }>): string {
  switch (now.kind) {
    case "building":
      return now.phase === "creating" ? "Creating…" : "Starting…";
    case "frozen":
      return now.freezing ? "Freezing…" : "Frozen";
    case "stopped":
      return "Stopped. Its files are kept; its processes ended.";
    case "leaving":
      return now.deleting ? "Deleting…" : "Stopping…";
    default:
      return unreachable(now);
  }
}

/** A phase, where it's worth saying. */
function phase(phase: Phase): string {
  switch (phase) {
    case "creating":
      return "creating";
    case "starting":
      return "starting";
    case "running":
      return "running";
    case "freezing":
      return "freezing";
    case "frozen":
      return "frozen";
    case "stopping":
      return "stopping";
    case "stopped":
      return "stopped";
    case "deleting":
      return "deleting";
    case "deleted":
      return "deleted";
    default:
      return unreachable(phase);
  }
}

/** What deleting a workspace would lose. */
function unsaved(unsaved: Unsaved): string {
  const files = unsaved.uncommitted === 0 ? null : plural(unsaved.uncommitted, { one: "1 uncommitted file", other: `${numbers.format(unsaved.uncommitted)} uncommitted files` });
  const commits = (() => {
    const unpushed = unsaved.unpushed;
    switch (unpushed.kind) {
      case "None":
        return null;
      case "Commits":
        return plural(unpushed.count, { one: "1 unpushed commit", other: `${numbers.format(unpushed.count)} unpushed commits` });
      case "NoUpstream":
        return "a branch that was never pushed";
      default:
        return unreachable(unpushed);
    }
  })();
  return lists.format([files, commits].filter((part) => part !== null));
}

/** What a kind of column is called before it has a name. */
function columnKind(kind: ColumnKind): string {
  switch (kind.kind) {
    case "Shell":
      return "Shell";
    case "Browser":
      return "Browser";
    default:
      return unreachable(kind);
  }
}

/** What a workspace may do to another, from inside, and what that means. */
function permission(permission: Permission): { label: string; title: string } {
  switch (permission) {
    case "view":
      return { label: "See", title: "See the workspace, its columns and what they're doing; every other permission includes this" };
    case "read_output":
      return { label: "Read", title: "Read what its terminals show" };
    case "send_input":
      return { label: "Type", title: "Type into its terminals: as much as running commands there" };
    case "manage_columns":
      return { label: "Columns", title: "Open, close and restart its columns" };
    case "publish_routes":
      return { label: "Publish", title: "Publish its ports as previews" };
    case "operate":
      return { label: "Start/stop", title: "Start, freeze, stop and rename it" };
    default:
      return unreachable(permission);
  }
}

/** What a shortcut does. */
function does(action: Action): string {
  switch (action.kind) {
    case "palette":
      return "Search and commands";
    case "new":
      return "New workspace";
    case "next-waiting":
      return "Next workspace that needs you";
    case "previews":
      return "Previews";
    case "keys":
      return "All shortcuts";
    case "workspace":
      return action.step === 1 ? "Next workspace" : "Previous workspace";
    case "open":
      return "Open the selected workspace";
    case "back":
      return "Back to the overview";
    case "last-workspace":
      return "The workspace you were in before";
    case "rename":
      return "Rename";
    case "freeze":
      return "Freeze or thaw";
    case "details":
      return "Details";
    case "column":
      return action.step === 1 ? "Next column" : "Previous column";
    case "column-at":
      return "Go to a column";
    case "move-column":
      return action.step === 1 ? "Move the column right" : "Move the column left";
    case "width":
      return "Change its width";
    case "zoom":
      return "Zoom it to fill the page, or put it back";
    case "find":
      return "Find in its output";
    case "leave":
      return "Leave the terminal for the page (⇧ goes back)";
    case "add-column":
      return "New column";
    case "label-column":
      return "Rename the column";
    case "close-column":
      return "End the column";
    default:
      return unreachable(action);
  }
}

/** Where a shortcut works. */
function group(group: Group): string {
  switch (group) {
    case "anywhere":
      return "Anywhere";
    case "workspaces":
      return "Workspaces";
    case "columns":
      return "Columns";
    default:
      return unreachable(group);
  }
}

/** Why a pasted theme can't be read. */
function themeProblem(problem: ThemeProblem): string {
  switch (problem.kind) {
    case "not-key-value":
      return `Line ${problem.line} isn't \`key = value\`.`;
    case "palette-entry":
      return `Line ${problem.line}: a palette entry is \`palette = 0=#1e1e2e\`.`;
    case "colour":
      return `Line ${problem.line}: ${problem.value} isn't a colour iglu reads; use hex, like #1e1e2e.`;
    case "missing":
      return "A theme needs at least a background and a foreground.";
    default:
      return unreachable(problem);
  }
}

/** Why iglud ended a terminal or the workspace's browser. */
function closed(why: Closed): string {
  switch (why) {
    case "NotRunning":
      return "The workspace isn't running";
    case "AccessEnded":
      return "Access to this workspace ended";
    case "HostFailed":
      return "The workspace's host couldn't attach to it";
    case "BrowserNotRunning":
      return "The browser isn't running; restart its column";
    case "BrowserUnreachable":
      return "The browser's display isn't answering; restart its column";
    default:
      return unreachable(why);
  }
}

const ONLY_YOU = "Only you can open it, signed in to iglu.";

/** A control's name with the keys that do the same, when there are any. */
const keyed = (text: string, keys: string | undefined) => (keys ? `${text} (${keys})` : text);

const settingsLink = (link: (text: string) => ComponentChildren) => <>Add an environment first, in {link("Settings")}.</>;

export const m = {
  refusal(refusal: Refusal): string {
    switch (refusal.from) {
      case "iglu":
        return reason(refusal.reason);
      case "nowhere":
        return "Couldn't reach iglu. Check the connection and try again.";
      case "elsewhere":
        return `Something other than iglu answered ${refusal.status}.`;
      case "garbled":
        return "iglu's reply wasn't what this page expected. Reload the page.";
      default:
        return unreachable(refusal);
    }
  },
  /** An error that isn't iglu's refusal, such as the browser's. */
  unexpected: (error: unknown) => (error instanceof Error ? error.message : String(error)),

  situation: { trouble, resting },
  phase,
  unsaved,
  columnKind,
  permission,
  history: {
    happened,
    /** What happened, and the workspace it was done from when one did it. */
    by: (what: string, via: { self: true } | { name: string } | null): ComponentChildren =>
      via === null ? what : "self" in via ? `${what} from this workspace` : <>{what} from <span translate={false}>{via.name}</span></>,
    heading: "History",
  },

  attention: {
    waiting: "needs you",
    done: (unseen: boolean) => (unseen ? "done · new" : "done"),
    working: "working",
    idle: "idle",
  },

  frame: {
    home: "iglu, overview",
    /** How many workspaces need you, the words hidden on a narrow screen. */
    waiting: (count: number) => (
      <>
        {numbers.format(count)}
        <span class="wide"> {plural(count, { one: "needs you", other: "need you" })}</span>
      </>
    ),
    waitingLabel: (count: number) => `${plural(count, { one: `${numbers.format(count)} needs you`, other: `${numbers.format(count)} need you` })}; go to the next`,
    waitingTitle: (keys: string | undefined) => keyed("Next workspace that needs you", keys),
    updated: "iglu was updated.",
    reload: "Reload",
    reconnecting: "Reconnecting…",
    previews: "Previews",
    previewsTitle: (keys: string | undefined) => keyed("Previews", keys),
    search: "Search and commands",
    searchWide: "Search…",
    newWorkspace: "New workspace",
    newWorkspaceTitle: (keys: string | undefined) => keyed("New workspace", keys),
    settings: (who: string) => `Settings, signed in as ${who}`,
    skip: "Skip to content",
    undo: "Undo",
    /** The tab's title: how many need you, the workspace open, and iglu. */
    tab: (count: number, page: string | null) => `${count ? `(${numbers.format(count)}) ` : ""}${page ? `${page} · ` : ""}iglu`,
  },

  notice: {
    waiting: (ws: string) => `${ws} needs you`,
    done: (ws: string) => `${ws} is done`,
    trouble: (ws: string) => `${ws} is in trouble`,
    troubleBody: (title: string, detail: string) => `${title}. ${detail}`,
  },

  notifications: {
    turnOn: "Turn on notifications",
    button: "Notifications",
    answer: (permission: NotificationPermission) => (permission === "granted" ? "Notifications are on." : "Notifications are blocked in this browser."),
  },

  welcome: {
    title: "Welcome to iglu",
    lede: "Each piece of work gets its own machine: a NixOS workspace whose terminals, agents and previews live in your browser.",
    environment: "Add an environment",
    environmentAbout: "The NixOS system your workspaces run, from a flake that uses iglu's workspace module.",
    project: "Add a project",
    projectAbout: "A repository to work on, and how its workspaces start. You can skip this: general has no repository.",
    workspace: "Start a workspace",
    workspaceAbout: "A branch to work on, with terminals to run your agents in.",
  },

  overview: {
    heading: "Workspaces",
    waiting: "Waiting on you",
    ready: (
      <>
        <b>Ready.</b> Start a workspace: a branch to work on, with terminals and the agents you start in it.
      </>
    ),
    newWorkspace: "New workspace",
    newIn: (project: string) => `New workspace in ${project}`,
    expand: (group: string) => `Expand ${group}`,
    collapse: (group: string) => `Collapse ${group}`,
    needYou: (count: number) => plural(count, { one: `${numbers.format(count)} needs you`, other: `${numbers.format(count)} need you` }),
    newProject: "New project",
    count: (count: number) => numbers.format(count),
  },

  card: {
    /** What a workspace without agent threads runs: "shell · server". */
    runs: (columns: readonly string[]) => columns.join(" · "),
    inSession: (session: string) => `in ${session}`,
    more: (count: number) => `+${numbers.format(count)} more`,
    stopped: "Its files are kept; its processes ended.",
    thaws: "Opening it thaws it",
  },

  preview: {
    onlyYou: ONLY_YOU,
    title: (url: string, asleep: string | null) => (asleep ? `${url} (the workspace is ${asleep}). ${ONLY_YOU}` : `${url}. ${ONLY_YOU}`),
    copy: "Copy link",
    copyLabel: (name: string) => `Copy the link to ${name}`,
    copied: (url: string) => `Copied ${url}. ${ONLY_YOU}`,
    notCopied: "This browser didn't let iglu copy the link.",
  },

  previews: {
    heading: "Previews",
    none: "Nothing is published.",
    who: "Only you can open your previews, signed in to iglu.",
    open: "Open workspace",
    openLabel: (ws: string) => `Open ${ws}`,
  },

  dialog: {
    close: "Close",
  },

  newWorkspace: {
    title: "New workspace",
    noEnvironment: settingsLink,
    project: "Project",
    branch: "Branch",
    branchHint: "A new branch, or one the remote has…",
    label: "Call it",
    labelHint: (repo: boolean) => (repo ? "Anything; the branch names it otherwise…" : "Anything, such as Tidy my notes…"),
    more: "More",
    name: "Name, in URLs and the CLI",
    nameHint: (made: string | null) => (made ? `${made}…` : "Picked for you…"),
    base: "A new branch starts from",
    baseHint: "The default branch…",
    opens: (columns: readonly string[]) => `Opens with ${lists.format(columns)}.`,
    cancel: (busy: boolean) => (busy ? "Close" : "Cancel"),
    create: (busy: boolean) => (busy ? "Creating…" : "Create"),
    /** What each input takes, said when iglu can't take what it has. */
    rules: {
      project: "Choose one of your projects.",
      branch: "A branch name is up to 100 characters Git accepts in one, such as fix/login.",
      base: "A branch name is up to 100 characters Git accepts in one, such as main.",
      label: "What you call it is one line of up to 80 characters, with no space at either end.",
      name: "A name is up to 63 lowercase letters, digits and dashes, starting with a letter and not ending with a dash.",
    } satisfies Rules<CreateWorkspace>,
    made: (ws: string) => `${ws} is being made; it's in the list.`,
  },

  newProject: {
    title: "New project",
    noEnvironment: settingsLink,
    repo: "Repository",
    repoHint: "https://github.com/you/app.git…",
    name: "Name",
    nameHint: "Named after the repository…",
    environment: "Environment",
    cancel: "Cancel",
    add: (busy: boolean) => (busy ? "Adding…" : "Add"),
    rules: {
      repo: "A repository is https://host/path, ssh://host/path or user@host:path.",
      name: "A name is up to 63 lowercase letters, digits and dashes, starting with a letter and not ending with a dash.",
      environment: "Choose one of your environments.",
    } satisfies Rules<CreateProject>,
  },

  environment: {
    add: "Add an environment",
    name: "Name",
    nameHint: "default…",
    source: "Flake",
    sourceHint: "github:you/iglu-env#default…",
    submit: (busy: boolean) => (busy ? "Adding…" : "Add"),
    building: (name: string) => `Building ${name}.`,
    rules: {
      name: "A name is up to 63 lowercase letters, digits and dashes, not starting or ending with a dash.",
      source: "A flake and one of its NixOS configurations, as flake#name, such as github:you/iglu-env#default.",
    } satisfies Rules<CreateEnvironment>,
  },

  workspace: {
    missing: (name: string | null) => (name ? `No workspace called ${name}.` : "No workspace called that."),
    overview: "Overview",
    list: "Workspaces",
    /** Keys a phone's keyboard lacks, by what each sends. */
    keyRow: "Terminal keys",
    ctrl: "Ctrl, for the next key",
    keys: { escape: "Escape", tab: "Tab", left: "Left", down: "Down", up: "Up", right: "Right", interrupt: "Interrupt (Ctrl+C)" },
    name: "Name, in URLs and the CLI",
    renameSubmit: "Rename",
    cancel: "Cancel",
    renameTitle: (keys: string | undefined) => keyed("Rename", keys),
    label: "What to call it",
    renameRules: {
      label: "What you call it is one line of up to 80 characters, with no space at either end.",
      name: "A name is up to 63 lowercase letters, digits and dashes, starting with a letter and not ending with a dash.",
    } satisfies Rules<RenameWorkspace & LabelWorkspace>,
    publish: "Publish",
    publishPort: "Publish a port",
    port: "port",
    portLabel: "Port to publish",
    notAPort: (text: string) => (text ? `${text} isn't a port number.` : "That isn't a port number."),
    portRules: { port: "A port is a number from 1 to 65535." } satisfies Rules<PublishPort>,
    freeze: "Freeze",
    freezeTitle: (keys: string | undefined) => keyed("Freeze", keys),
    thaw: "Thaw",
    thawTitle: (keys: string | undefined) => keyed("Thaw", keys),
    start: "Start",
    details: "Details",
    detailsTitle: (keys: string | undefined) => keyed("Details", keys),
    /** A trouble on one line, above the columns. */
    troubleLine: (title: string, detail: string) => (
      <>
        <b>{title}.</b> {detail}
      </>
    ),
    branchTitle: (branch: string) => `On ${branch}`,
    ahead: (count: number) => `↑${numbers.format(count)}`,
    behind: (count: number) => `↓${numbers.format(count)}`,
    changed: (count: number) => `${numbers.format(count)} changed`,
    conflicted: (count: number) => `${numbers.format(count)} conflicted`,
    gitMarks: (marks: readonly string[]) => marks.join(" · "),
    listening: {
      label: (port: number, process: string | null) => (process ? `Publish :${port} (${process})` : `Publish :${port}`),
      title: (process: string | null, column: string | null) =>
        `${process || "Something"} is listening${column ? ` in ${column}` : ""}. Publish it.`,
      unreachable: (process: string | null, port: number) =>
        `${process || "Something"} listens on :${port} only where previews can't reach. Bind 0.0.0.0 or 127.0.0.1 to publish it.`,
    },
    columns: "Columns",
    sessionTitle: (label: string, session: string) => `${label} (session ${session})`,
    rang: "It rang the bell",
    rangLabel: "(rang the bell)",
    addColumn: "Add a column",
    addColumnTitle: (keys: string | undefined) => keyed("Add a column", keys),
    shell: "Shell",
    openingShell: "Opening a shell…",
    browser: "Browser",
    browserOpen: "(open)",
    openingBrowser: "Opening the browser…",
    columnName: (title: string) => `Name of ${title}`,
    columnRules: { label: "A column's name is one line of up to 40 characters, with no space at either end." } satisfies Rules<LabelColumn>,
    find: {
      label: (title: string) => `Find in ${title}`,
      placeholder: "Find",
      finding: "Finding…",
      none: "No matches",
      /** Which match of how many, counted from the oldest; `limited` when there are more than were counted. */
      count: (count: number, limited: boolean) => `${numbers.format(count)}${limited ? "+" : ""}`,
      at: (index: number, count: number, limited: boolean) => `${numbers.format(index)} of ${numbers.format(count)}${limited ? "+" : ""}`,
      earlier: "Earlier match",
      earlierTitle: "Earlier (↩)",
      later: "Later match",
      laterTitle: "Later (⇧↩)",
      close: "Close find",
      closeTitle: "Close (esc)",
    },
    offer: {
      label: "A program asks to open an address",
      open: "Open",
      inWorkspace: "Open in the workspace's browser",
      nowhere: "Only the workspace's browser reaches this, and its environment has none.",
      dismiss: "Don't open it",
    },
    end: (title: string) => `End ${title}`,
    keep: "Keep",
    openShell: "Open a shell",
    column: (title: string) => `Column ${title}`,
    renameColumnTitle: (session: string | null, keys: string | undefined) => keyed(`${session ? `Session ${session}. ` : ""}Double-click to rename`, keys),
    programTitle: "What the program here calls itself",
    threads: (count: number) => plural(count, { one: "1 thread", other: `${numbers.format(count)} threads` }),
    threadsIn: (title: string) => `Threads in ${title}`,
    newThread: "New thread",
    zoom: (title: string) => `Zoom ${title}`,
    unzoom: (title: string) => `Unzoom ${title}`,
    zoomTitle: (keys: string | undefined) => keyed("Zoom to fill the page", keys),
    unzoomTitle: (keys: string | undefined) => keyed("Unzoom: put it back", keys),
    widthTitle: (keys: string | undefined) => keyed("Width", keys),
    widthLabel: (title: string, width: string) => `Width of ${title}: ${width}`,
    moveLeft: (title: string) => `Move ${title} left`,
    moveRight: (title: string) => `Move ${title} right`,
    endTitle: (keys: string | undefined) => keyed("End this column", keys),
    ended: "Ended.",
    restart: "Restart",
    cloning: (repo: ComponentChildren, project: string) => (
      <>
        It clones {repo}, as {project} had it when this workspace was made. If that's wrong, fix the project's repository, then delete this workspace and make a new one.
      </>
    ),
    updateNewest: "Update to the newest build",
    openProject: (project: string) => `Open ${project}`,
    stop: "Stop",
    deleteAsk: "Delete…",
    thawHint: (keys: string | undefined) => (keys ? `${keys} to thaw` : undefined),
    loss: {
      noCheckout: "Its files are lost.",
      unchecked: (phase: string) => `It's ${phase}, so unpushed work can't be checked. Its files are lost.`,
      unknown: "Its files are lost; push your work first.",
      unsaved: (what: string) => `It has ${what}, which would be lost.`,
      none: "Everything is committed and pushed.",
    },
    info: {
      label: "Details",
      heading: "Workspace",
      close: "Close details",
      project: "Project",
      repo: "Repository",
      branch: "Branch",
      environment: "Environment",
      memory: "Memory in use",
      created: "Created",
      none: "—",
      memoryAmount: gibibytes,
      saveOpening: "Open new workspaces like this",
      newer: (environment: string) => `A newer build of ${environment}`,
      newerAbout: "Updating upgrades this workspace to it and restarts it. Its files stay; what's running ends.",
      update: "Update",
      previews: "Previews",
    },
    confirmDelete: (ws: string, loss: string) => `Delete ${ws}? ${loss}`,
    delete: "Delete",
    keepIt: "Keep it",
    access: {
      heading: "Access from inside",
      about: (
        <>
          What <code>iglu</code> can do when run in this workspace. Any program here can use it.
        </>
      ),
      self: "This workspace",
      gone: "A deleted workspace",
      may: (ws: string) => `What it may do to ${ws}`,
      add: "Let it see another workspace",
      addHint: "Let it see another workspace…",
      refused: (why: string) => `That access couldn't be changed. ${why}`,
    },
    unpublish: "Unpublish",
    unpublishSure: "Unpublish for good",
    unpublishTitle: "Its name is never reused",
    updating: (ws: string, environment: string) => `Updating ${ws} to the newest build of ${environment}. It restarts once it's ready.`,
    nothingWaiting: "Nothing needs you.",
    noLast: "There's no other workspace to go back to yet.",
    notOurs: (column: string) => `${column} was opened inside the workspace, not by iglu, so it can't be renamed.`,
    published: (port: number, route: string, listening: boolean) =>
      listening
        ? `Published :${port} as ${route}; its link is beside the name.`
        : `Published :${port} as ${route}. Nothing listens there yet; it opens once something does.`,
    openingSaved: (project: string) => `New workspaces in ${project} open like this.`,
  },

  project: {
    missing: (name: string | null) => (name ? `No project called ${name}.` : "No project called that."),
    /** Where a project's workspaces come from: its repository, and its environment. */
    source: (repo: string | null, environment: string) => `${repo ?? "no repository"} · ${environment}`,
    settings: "Settings",
    newWorkspace: "New workspace",
    empty: "No workspaces yet.",
    idle: (kind: IdleRule["kind"]) => {
      switch (kind) {
        case "Default":
          return "Freeze as usual";
        case "Never":
          return "Stay awake";
        case "After":
          return "Freeze after…";
        default:
          return unreachable(kind);
      }
    },
    notAPort: (part: string) => `${part} isn't a port number.`,
    wholeMinutes: "Freeze after a whole number of minutes.",
    saved: "Saved.",
    settingsFor: (project: string) => `Settings for ${project}`,
    stale: "These settings were saved elsewhere since you started editing, so yours can't be saved over them.",
    showSaved: "Show the saved settings",
    name: "Name",
    repo: "Repository",
    repoHint: "None…",
    environment: "Environment",
    ports: "Previews",
    portsHint: "3000, 5173…",
    whenUnused: "When unused",
    minutes: "Minutes",
    opening: "New workspaces open with",
    widthOf: (column: string) => `Width of ${column}`,
    moveLeft: "Move left",
    moveRight: "Move right",
    remove: (column: string) => `Remove ${column}`,
    save: (busy: boolean) => (busy ? "Saving…" : "Save"),
    rules: {
      name: "A name is up to 63 lowercase letters, digits and dashes, starting with a letter and not ending with a dash.",
      repo: "A repository is https://host/path, ssh://host/path or user@host:path.",
      environment: "Choose one of your environments.",
      ports: "Previews are up to 16 ports from 1 to 65535, except the browser's 5900 and 9222.",
      idle: "Choose when its workspaces freeze.",
      opening: "New workspaces open with at most 16 columns, and one browser.",
    } satisfies Rules<ChangeProject>,
    deleteFirst: (project: string) => `Delete its workspaces to remove ${project}.`,
    confirmRemove: (project: string) => `Remove ${project}? Its settings go; nothing else does.`,
    removeSure: "Remove",
    keepIt: "Keep it",
    removeProject: "Remove project",
  },

  settings: {
    heading: "Settings",
    environments: "Environments",
    build: (latest: { status: "Building" } | { status: "Ready"; at: string } | { status: "Failed" } | null) => {
      if (latest === null) return "never built";
      switch (latest.status) {
        case "Building":
          return "building…";
        case "Ready":
          return `built ${latest.at}`;
        case "Failed":
          return "build failed";
        default:
          return unreachable(latest);
      }
    },
    rebuild: "Rebuild",
    log: "Build log",
    projects: "Projects",
    workspaces: (count: number) => plural(count, { one: "1 workspace", other: `${numbers.format(count)} workspaces` }),
    secrets: "Secrets",
    secretsAbout: "Every workspace you own gets these. Values are never shown again.",
    target: (kind: SecretTarget["kind"]) => {
      switch (kind) {
        case "env":
          return "Environment variable";
        case "file":
          return "File in home";
        case "git_credential":
          return "Git credential";
        default:
          return unreachable(kind);
      }
    },
    variable: "Variable",
    variableHint: "CLAUDE_CODE_OAUTH_TOKEN…",
    path: "Path",
    pathHint: ".ssh/id_ed25519…",
    host: "Host",
    hostHint: "github.com…",
    username: "Username",
    usernameHint: "x-access-token…",
    deleteSecret: (name: string) => `Delete ${name}`,
    keep: "Keep",
    delete: "Delete",
    addSecret: "Add or replace a secret",
    secretName: "Name",
    secretNameHint: "anthropic…",
    goesTo: "Goes to",
    value: "Value",
    save: (busy: boolean) => (busy ? "Saving…" : "Save"),
    secretSaved: (name: string) => `Saved ${name}. New terminals get it.`,
    secretRules: {
      name: "A secret's name is up to 64 lowercase letters, digits, dashes, underscores and dots.",
      target: "Where it goes: a variable is capital letters, digits and underscores, not one iglu sets; a path stays inside your home; a host is a name such as github.com.",
      value: "A value is up to 64 KiB.",
    } satisfies Rules<PutSecret & SecretPath>,
    keyboard: "Keyboard",
    prefix: "Prefix, to reach iglu from a terminal",
    prefixRecording: "Press a Ctrl chord for the prefix",
    prefixLabel: (chord: string) => `Prefix: ${chord}. Change it`,
    prefixPress: "Press a Ctrl chord…",
    prefixRefused: "Use Ctrl with a letter, Space, [, ] or \\.",
    altMoves: (keys: string) => `${keys} move between columns and workspaces, even in a terminal`,
    optionAsMeta: "Option as Meta in terminals",
    meta: (meta: OptionAsMeta) => {
      switch (meta) {
        case "Left":
          return "Left Option";
        case "Both":
          return "Both Options";
        case "Off":
          return "Neither";
        default:
          return unreachable(meta);
      }
    },
    terminal: "Terminal",
    colours: "Colours",
    ownColours: "iglu's own",
    light: "Light",
    dark: "Dark",
    pastedColours: "Pasted from Ghostty",
    pasteAbout: (
      <>
        A theme in Ghostty's format: a theme file, or <code>ghostty +show-config</code>
      </>
    ),
    themeProblem: (problem: ThemeProblem) => `${themeProblem(problem)} Until it reads, terminals keep iglu's own colours.`,
    font: "Font",
    fontRefused: "A font's name is letters, digits, spaces, dots, dashes and underscores.",
    fontTrouble: (font: string, missing: boolean) =>
      missing ? `${font} isn't installed in this browser, so terminals show JetBrains Mono.` : `${font} isn't monospaced, so terminals show JetBrains Mono.`,
    size: "Size",
    pixels: (size: number) => `${numbers.format(size)}px`,
    account: "Account",
    look: "Look",
    lookName: (look: Look) => {
      switch (look) {
        case "Auto":
          return "System";
        case "Dark":
          return "Polar night";
        case "Light":
          return "Snowfield";
        default:
          return unreachable(look);
      }
    },
    signOut: "Sign out",
    notKept: (why: string) => `That setting couldn't be kept. ${why}`,
  },

  palette: {
    label: "Search and commands",
    search: "Search",
    placeholder: "Workspaces, previews, commands…",
    results: "Results",
    none: "No matches.",
    newWorkspace: "New workspace",
    newProject: "New project",
    nextWaiting: "Next workspace that needs you",
    overview: "Overview",
    previews: "Previews",
    settings: "Settings",
    settingsAbout: "Environments, secrets, keyboard",
    keys: "Keyboard shortcuts",
    back: (ws: string) => `Back to ${ws}`,
    backAbout: "The workspace you were in before",
    rename: (ws: string) => `Rename ${ws}`,
    details: (ws: string) => `Details of ${ws}`,
    project: (project: string) => `Go to project ${project}`,
    projectAbout: (ws: string) => `Where ${ws} came from, and its settings`,
    copyLink: (route: string) => `Copy the link to ${route}`,
    shell: (ws: string) => `Shell in ${ws}`,
    browser: (ws: string) => `Browser in ${ws}`,
    publish: (ws: string) => `Publish a port of ${ws}`,
    freeze: (ws: string) => `Freeze ${ws}`,
    thaw: (ws: string) => `Thaw ${ws}`,
    stop: (ws: string) => `Stop ${ws}`,
    stopAbout: "Its files stay; starting it runs it again",
    start: (ws: string) => `Start ${ws}`,
    update: (ws: string) => `Update ${ws}`,
    updateAbout: (environment: string) => `To the newest build of ${environment}, restarting it`,
    delete: (ws: string) => `Delete ${ws}…`,
    deleteAbout: "Asks first",
    branch: (branch: string) => `⎇ ${branch}`,
    projectKind: "Project",
    projectSettings: (project: string) => `Settings of ${project}`,
    projectSettingsAbout: "Repository, environment, previews, how workspaces open",
    openPreview: (route: string) => `Open ${route}`,
    previewAbout: (ws: string, port: number) => `${ws} :${port}`,
    look: (look: Look) => {
      switch (look) {
        case "Auto":
          return "Look: match the system";
        case "Dark":
          return "Look: Polar night";
        case "Light":
          return "Look: Snowfield";
        default:
          return unreachable(look);
      }
    },
    signOut: "Sign out",
    goTo: (column: string) => `Go to column ${column}`,
    session: (session: string) => `session ${session}`,
    end: (column: string) => `End ${column}`,
    zoom: (column: string) => `Zoom ${column}`,
    zoomAbout: "Fill the page with it",
    unzoom: (column: string) => `Unzoom ${column}`,
    unzoomAbout: "Put it back as it was",
    width: (column: string) => `Change ${column}'s width`,
    widthAbout: (width: string) => `It's ${width} of the strip`,
    moveLeft: (column: string) => `Move ${column} left`,
    moveRight: (column: string) => `Move ${column} right`,
    renameColumn: (column: string) => `Rename column ${column}`,
    find: (column: string) => `Find in ${column}`,
    findAbout: "Search its output",
    restart: (column: string) => `Restart ${column}`,
  },

  keys: {
    title: "Keyboard shortcuts",
    does,
    group,
    /** How the keys reach iglu from a terminal, with the chords drawn by `chord`. */
    about: (altMoves: boolean, alts: ComponentChildren, prefix: ComponentChildren) => (
      <>
        {altMoves ? (
          <>Terminals keep every key but {alts}, which move between columns and workspaces; Settings can give those back. </>
        ) : (
          "Terminals keep every key. "
        )}
        To reach iglu from one, press {prefix}, then a key. Pressing it twice sends {prefix} to the terminal. On the overview, the keys work without it.
      </>
    ),
    toColumn: "Go to a column",
    openLink: "Open a link in its output",
    waiting: "iglu is waiting for a key",
    sendPrefix: (prefix: string) => `Send ${prefix} to the terminal`,
    cancel: "Cancel",
  },

  terminal: {
    label: (title: string) => `Terminal, ${title}`,
    description: (prefix: string) => `${prefix}, then Tab leaves the terminal; ${prefix}, then ? lists shortcuts.`,
    reconnecting: (why: Closed | null) => (why ? `Reconnecting… (${closed(why)})` : "Reconnecting…"),
    copyRefused: "What a program copied couldn't reach your clipboard.",
    tooLarge: (file: string) => (file ? `${file} is larger than 16 MB, so it can't be pasted.` : "That file is larger than 16 MB, so it can't be pasted."),
    sending: (files: readonly string[]) => (files.length === 1 ? `Sending ${files[0]}…` : `Sending ${numbers.format(files.length)} files…`),
    notPasted: (why: string) => `That couldn't be pasted. ${why}`,
  },

  browser: {
    label: (title: string) => `Browser, ${title}`,
    description: (prefix: string) => `${prefix}, then Tab leaves the browser; ${prefix}, then ? lists shortcuts.`,
    opening: "Opening the browser…",
    retrying: (why: Closed | null) => (why ? `${closed(why)}. Trying again…` : "Reconnecting…"),
    copyRefused: "The browser's selection couldn't be copied here.",
    kept: (paths: readonly string[], dir: string) =>
      paths.length === 1 ? `Kept in the workspace at ${paths[0]}, for the browser to open.` : `Kept ${numbers.format(paths.length)} files in the workspace, in ${dir}, for the browser to open.`,
    downloaded: "Downloaded",
    downloadedTo: (name: string | null, path: string) =>
      name === null ? (
        <>
          Downloaded to <span translate={false}>{path}</span>.
        </>
      ) : (
        <>
          Downloaded <span translate={false}>{name}</span>.
        </>
      ),
    dismiss: "Dismiss",
    save: "Save to this computer",
  },

  start: {
    failed: (why: string) => `The console couldn't start. ${why}`,
  },
};

/** The console's words. Another language is another object of this type. */
export type Messages = typeof m;
