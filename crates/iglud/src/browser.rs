//! Showing a workspace's browser in the console, and letting the person use it.
//!
//! The browser is Chromium, run by a browser column, listening for Chrome's
//! debugging protocol (CDP) on the workspace's loopback (see `iglu-guest
//! browser`).
//! iglud reaches it through a port tunnel, like the preview gateway does, and
//! speaks the protocol for the console: it shows one tab at a time as a
//! screencast, turns what the person does into input events, and keeps the
//! console told about the tabs. Agents in the workspace drive the same
//! browser through the same port, so the person watches what they do.
//!
//! [`Screen`] is the protocol, without input or output: what to send the
//! browser and the console for each thing either says. [`relay`] carries it.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{Message as Console, WebSocket};
use base64::Engine;
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use http_body_util::{BodyExt, Empty, Limited};
use hyper_util::rt::TokioIo;
use iglu_api::browser::{
    BrowserEvent, BrowserInput, BrowserSize, BrowserTab, DialogKind, KeyAction, KeyInput,
    Modifiers, MouseAction, MouseButton, MouseInput,
};
use iglu_domain::column::BROWSER_DEBUG_PORT;
use iglu_domain::id::WorkspaceId;
use iglu_domain::port::GuestPort;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message as Devtools;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;

use crate::app::{App, Caller};
use crate::hosts::HostClient;
use crate::model::WorkspaceRecord;
use crate::terminal::{LEASE, Use, refusal, still_allowed};

/// How long opening waits for a browser that's starting.
const STARTING: Duration = Duration::from_secs(15);

/// How long opening waits for a frozen workspace to thaw.
const THAW: Duration = Duration::from_secs(60);

/// JPEG quality of the screen: sharp enough for text, small enough to keep up.
const QUALITY: u8 = 70;

/// The most commands left unanswered at once. Chrome answers each at once;
/// what's beyond this from a browser that doesn't goes unrecorded, and its
/// answer, if one comes, is ignored.
const MAX_PENDING: usize = 256;

/// The most tabs shown in the tab strip; a page that opens more is a
/// runaway, and the rest go unlisted.
const MAX_TABS: usize = 100;

/// How long one send, to either side, may take before the relay gives up on
/// it: a peer that stops reading would otherwise hold off the lease.
const SEND: Duration = Duration::from_secs(10);

/// The largest message taken from the browser.
const MAX_MESSAGE: usize = 16 * 1024 * 1024;

/// The most of the browser's version report read, and how long asking takes.
const VERSION_BYTES: usize = 64 * 1024;
const VERSION_TIME: Duration = Duration::from_secs(5);

/// Reads the selection where the person would copy from: in a text field,
/// its selected text; elsewhere, the page's.
const SELECTION: &str = "(() => { const e = document.activeElement; \
     if (e && typeof e.selectionStart === 'number' && typeof e.value === 'string') \
     return e.value.slice(e.selectionStart, e.selectionEnd); \
     return String(getSelection()); })()";

/// Keeps Chrome from opening its own context menu, which a screencast never
/// shows and which, invisible, holds the browser until it's closed. The
/// page still gets the right click, so a page's own menu works.
const NO_NATIVE_MENU: &str =
    "addEventListener('contextmenu', (e) => { if (!e.defaultPrevented) e.preventDefault(); })";

/// Something to send: a CDP message to the browser, or an event or an
/// image of the screen to the console.
#[derive(Debug, PartialEq)]
pub enum Out {
    Devtools(String),
    Event(BrowserEvent),
    Image(Bytes),
}

/// A message from the browser: the answer to something sent, or an event.
/// Events from a tab carry the session attached to it.
#[derive(Debug, Deserialize)]
pub struct Incoming {
    id: Option<u64>,
    method: Option<String>,
    #[serde(default)]
    params: Value,
    #[serde(default)]
    result: Value,
    error: Option<Value>,
    #[serde(rename = "sessionId")]
    session: Option<String>,
}

/// What an answer from the browser is for.
#[derive(Debug)]
enum Pending {
    /// Discovering targets, which reports each tab there is first.
    Discovered,
    /// Attaching to the tab, which gives the session to drive it with.
    Attached {
        tab: String,
    },
    /// Reading the tab's history, to step back or forward in it.
    History {
        step: i64,
    },
    /// Opening a tab, to show it.
    Opened,
    Copied,
    /// Reading a tab's title and address, which Chrome doesn't report as a
    /// page's title settles.
    Info,
    /// Answering a tab's dialog, which may already be gone.
    Answered {
        tab: String,
    },
}

/// The tab shown, and its session once attached.
#[derive(Debug)]
struct Shown {
    tab: String,
    session: Option<String>,
}

/// The console's view of the browser: which tab it shows, at what size.
#[derive(Debug)]
pub struct Screen {
    next: u64,
    pending: HashMap<u64, Pending>,
    tabs: Vec<BrowserTab>,
    shown: Option<Shown>,
    size: BrowserSize,
    /// The page size the console was last told.
    told: Option<(u32, u32)>,
    discovered: bool,
    /// The dialog each tab has open, as last reported. Chrome reports a
    /// dialog once, to the session attached when it opens, so one opened
    /// before a switch is shown again on coming back.
    dialogs: HashMap<String, BrowserEvent>,
}

impl Screen {
    /// Starts showing the browser at `size`: first, finding its tabs.
    pub fn new(size: BrowserSize) -> (Self, Vec<Out>) {
        let mut screen = Self {
            next: 1,
            pending: HashMap::new(),
            tabs: Vec::new(),
            shown: None,
            size: fitted(size),
            told: None,
            discovered: false,
            dialogs: HashMap::new(),
        };
        let first = screen.call(
            "Target.setDiscoverTargets",
            json!({ "discover": true }),
            None,
            Some(Pending::Discovered),
        );
        (screen, vec![first])
    }

    /// What to send for what the person did.
    pub fn input(&mut self, input: BrowserInput) -> Vec<Out> {
        let mut out = Vec::new();
        match input {
            BrowserInput::Resize(size) => {
                self.size = fitted(size);
                if let Some(session) = self.session() {
                    out.extend(self.cast(&session));
                }
            }
            BrowserInput::Mouse(mouse) => {
                out.extend(self.on_page("Input.dispatchMouseEvent", mouse_event(&mouse), None));
            }
            BrowserInput::Key(key) => {
                out.extend(self.on_page("Input.dispatchKeyEvent", key_event(&key), None));
            }
            BrowserInput::Text { text } => {
                out.extend(self.on_page("Input.insertText", json!({ "text": text }), None));
            }
            BrowserInput::Copy => {
                let params = json!({ "expression": SELECTION, "returnByValue": true });
                out.extend(self.on_page("Runtime.evaluate", params, Some(Pending::Copied)));
            }
            BrowserInput::Navigate { address: typed } => {
                let params = json!({ "url": address(&typed) });
                out.extend(self.on_page("Page.navigate", params, None));
            }
            BrowserInput::Back => out.extend(self.step(-1)),
            BrowserInput::Forward => out.extend(self.step(1)),
            BrowserInput::Reload => out.extend(self.on_page("Page.reload", json!({}), None)),
            // A tab that's gone, closed by an agent as it was chosen, isn't.
            BrowserInput::Show { tab } if self.tabs.iter().any(|t| t.id == tab) => {
                out.extend(self.show(tab));
            }
            BrowserInput::Show { .. } => out.push(self.tabs_event()),
            BrowserInput::Open => out.push(self.call(
                "Target.createTarget",
                json!({ "url": "about:blank" }),
                None,
                Some(Pending::Opened),
            )),
            BrowserInput::Close { tab } => {
                out.push(self.call("Target.closeTarget", json!({ "targetId": tab }), None, None));
            }
            BrowserInput::Answer { accept, text } => {
                let params = json!({ "accept": accept, "promptText": text.unwrap_or_default() });
                if let Some(tab) = self.shown.as_ref().map(|s| s.tab.clone()) {
                    let answered = Pending::Answered { tab };
                    out.extend(self.on_page("Page.handleJavaScriptDialog", params, Some(answered)));
                }
            }
        }
        out
    }

    /// What to send as time passes: the shown tab's title again, which a
    /// page may change itself without the browser saying.
    pub fn tick(&mut self) -> Vec<Out> {
        match self.shown.as_ref().map(|s| s.tab.clone()) {
            Some(tab) => vec![self.info(&tab)],
            None => Vec::new(),
        }
    }

    /// What to send for what the browser said.
    pub fn devtools(&mut self, message: Incoming) -> Vec<Out> {
        if let Some(id) = message.id {
            return match self.pending.remove(&id) {
                Some(pending) if message.error.is_none() => self.answered(pending, &message.result),
                Some(pending) => {
                    tracing::debug!(?pending, error = ?message.error, "the browser refused a command");
                    self.refused(pending)
                }
                None => Vec::new(),
            };
        }
        let Some(method) = message.method else {
            return Vec::new();
        };
        if let Some(event) = method.strip_prefix("Target.") {
            return self.target_event(event, message.params);
        }
        if method == "Page.screencastFrame" {
            return self.frame(message.session, message.params);
        }
        // The rest come from a tab's session; only the shown tab's matter.
        if message.session.is_none() || message.session != self.session() {
            return Vec::new();
        }
        self.page_event(&method, message.params)
    }

    /// The browser's tabs coming, changing and going.
    fn target_event(&mut self, event: &str, params: Value) -> Vec<Out> {
        match event {
            "targetCreated" => {
                let Ok(TargetEvent { target }) = serde_json::from_value(params) else {
                    return Vec::new();
                };
                if target.kind != "page" {
                    return Vec::new();
                }
                let opened_here = target.opener.is_some()
                    && target.opener.as_deref() == self.shown.as_ref().map(|s| s.tab.as_str());
                let id = target.id.clone();
                self.tabs.retain(|t| t.id != id);
                if self.tabs.len() >= MAX_TABS {
                    return Vec::new();
                }
                self.tabs.push(target.tab());
                // A link that opens a new tab opens it in front, as it would
                // in any browser; so does the first tab of a browser with none.
                if self.discovered && (opened_here || self.shown.is_none()) {
                    self.show(id)
                } else {
                    vec![self.tabs_event()]
                }
            }
            "targetInfoChanged" => {
                let Ok(TargetEvent { target }) = serde_json::from_value(params) else {
                    return Vec::new();
                };
                let Some(tab) = self.tabs.iter_mut().find(|t| t.id == target.id) else {
                    return Vec::new();
                };
                *tab = target.tab();
                vec![self.tabs_event()]
            }
            "targetDestroyed" => {
                let Some(id) = params["targetId"].as_str() else {
                    return Vec::new();
                };
                self.tabs.retain(|t| t.id != id);
                self.dialogs.remove(id);
                if self.shown.as_ref().is_some_and(|s| s.tab == id) {
                    self.shown = None;
                    self.show_another()
                } else {
                    vec![self.tabs_event()]
                }
            }
            // The tab crashed, or something else closed the session.
            "detachedFromTarget" => {
                let detached = params["sessionId"].as_str();
                match &self.shown {
                    Some(shown) if detached.is_some() && shown.session.as_deref() == detached => {
                        let tab = shown.tab.clone();
                        self.shown = None;
                        self.show(tab)
                    }
                    _ => Vec::new(),
                }
            }
            _ => Vec::new(),
        }
    }

    /// An image of a tab, which is acknowledged whichever tab it's of, so
    /// the browser sends the next, and shown if it's of the shown tab.
    fn frame(&mut self, session: Option<String>, params: Value) -> Vec<Out> {
        let Some(session) = session else {
            return Vec::new();
        };
        // Acknowledged once the console has it, so the browser sends images
        // as fast as the console takes them.
        let ack = json!({ "sessionId": params["sessionId"] });
        let ack = self.call("Page.screencastFrameAck", ack, Some(&session), None);
        if self.session().as_deref() != Some(session.as_str()) {
            return vec![ack];
        }
        let Ok(frame) = serde_json::from_value::<Frame>(params) else {
            return vec![ack];
        };
        let Ok(image) = base64::engine::general_purpose::STANDARD.decode(frame.data) else {
            return vec![ack];
        };
        let mut out = Vec::new();
        let page = (pixels(frame.metadata.width), pixels(frame.metadata.height));
        if self.told != Some(page) {
            self.told = Some(page);
            out.push(Out::Event(BrowserEvent::Viewport {
                width: page.0,
                height: page.1,
            }));
        }
        out.push(Out::Image(image.into()));
        out.push(ack);
        out
    }

    /// What the shown tab's page does.
    fn page_event(&mut self, method: &str, params: Value) -> Vec<Out> {
        let Some(tab) = self.shown.as_ref().map(|s| s.tab.clone()) else {
            return Vec::new();
        };
        match method {
            "Page.frameStartedLoading" | "Page.frameStoppedLoading" => {
                if params["frameId"] != tab.as_str() {
                    return Vec::new();
                }
                let loading = method == "Page.frameStartedLoading";
                let mut out = vec![Out::Event(BrowserEvent::Loading { loading })];
                if !loading {
                    out.push(self.info(&tab));
                }
                out
            }
            // A page that changes its address itself, as an app does.
            "Page.navigatedWithinDocument" => vec![self.info(&tab)],
            "Page.javascriptDialogOpening" => {
                let Ok(dialog) = serde_json::from_value::<Dialog>(params) else {
                    return Vec::new();
                };
                let event = BrowserEvent::Dialog {
                    kind: dialog.kind,
                    message: dialog.message,
                    default: dialog.default.unwrap_or_default(),
                };
                self.dialogs.insert(tab, event.clone());
                vec![Out::Event(event)]
            }
            "Page.javascriptDialogClosed" => {
                self.dialogs.remove(&tab);
                vec![Out::Event(BrowserEvent::DialogClosed)]
            }
            _ => Vec::new(),
        }
    }

    fn answered(&mut self, pending: Pending, result: &Value) -> Vec<Out> {
        match pending {
            Pending::Discovered => {
                self.discovered = true;
                let mut out = vec![self.tabs_event()];
                if self.shown.is_none() {
                    out.extend(self.show_another());
                }
                out
            }
            Pending::Attached { tab } => {
                let Some(session) = result["sessionId"].as_str().map(str::to_owned) else {
                    return Vec::new();
                };
                match &mut self.shown {
                    Some(shown) if shown.tab == tab && shown.session.is_none() => {
                        shown.session = Some(session.clone());
                        // The script goes with this session: pages aren't
                        // changed for agents when no one's watching.
                        let menu = json!({ "source": NO_NATIVE_MENU });
                        let now = json!({ "expression": NO_NATIVE_MENU });
                        let mut out = vec![
                            self.call("Page.enable", json!({}), Some(&session), None),
                            self.call(
                                "Page.addScriptToEvaluateOnNewDocument",
                                menu,
                                Some(&session),
                                None,
                            ),
                            self.call("Runtime.evaluate", now, Some(&session), None),
                        ];
                        out.extend(self.cast(&session));
                        out
                    }
                    // Another tab was asked for meanwhile, or this one twice
                    // and it's attached already.
                    _ => vec![self.call(
                        "Target.detachFromTarget",
                        json!({ "sessionId": session }),
                        None,
                        None,
                    )],
                }
            }
            Pending::History { step } => {
                let Ok(history) = serde_json::from_value::<History>(result.clone()) else {
                    return Vec::new();
                };
                let Some(entry) = history
                    .current
                    .checked_add(step)
                    .and_then(|i| usize::try_from(i).ok())
                    .and_then(|i| history.entries.get(i))
                else {
                    return Vec::new();
                };
                self.on_page(
                    "Page.navigateToHistoryEntry",
                    json!({ "entryId": entry.id }),
                    None,
                )
                .into_iter()
                .collect()
            }
            Pending::Opened => match result["targetId"].as_str() {
                Some(tab) => self.show(tab.to_owned()),
                None => Vec::new(),
            },
            Pending::Info => {
                let Ok(TargetEvent { target }) = serde_json::from_value(result.clone()) else {
                    return Vec::new();
                };
                match self.tabs.iter_mut().find(|t| t.id == target.id) {
                    Some(tab) if *tab != target.tab() => {
                        *tab = target.tab();
                        vec![self.tabs_event()]
                    }
                    _ => Vec::new(),
                }
            }
            // The page reports the dialog closing.
            Pending::Answered { .. } => Vec::new(),
            Pending::Copied => match result["result"]["value"].as_str() {
                Some(text) if !text.is_empty() => vec![Out::Event(BrowserEvent::Copied {
                    text: text.to_owned(),
                })],
                _ => Vec::new(),
            },
        }
    }

    /// What to send when the browser refuses a command.
    fn refused(&mut self, pending: Pending) -> Vec<Out> {
        match pending {
            // The tab went as it was attached to: show another.
            Pending::Attached { tab } => match &self.shown {
                Some(shown) if shown.tab == tab && shown.session.is_none() => {
                    self.shown = None;
                    self.show_another()
                }
                _ => Vec::new(),
            },
            // The dialog was answered already, by something else.
            Pending::Answered { tab } => {
                self.dialogs.remove(&tab);
                vec![Out::Event(BrowserEvent::DialogClosed)]
            }
            Pending::Discovered
            | Pending::History { .. }
            | Pending::Opened
            | Pending::Copied
            | Pending::Info => Vec::new(),
        }
    }

    /// Shows `tab`: attaches to it, and leaves the one shown before.
    fn show(&mut self, tab: String) -> Vec<Out> {
        if self.shown.as_ref().is_some_and(|s| s.tab == tab) {
            return Vec::new();
        }
        let mut out = Vec::new();
        if let Some(session) = self.shown.take().and_then(|s| s.session) {
            out.push(self.call(
                "Target.detachFromTarget",
                json!({ "sessionId": session }),
                None,
                None,
            ));
        }
        // Brought to the front for agents too, so the tab they find is the
        // one the person sees.
        out.push(self.call(
            "Target.activateTarget",
            json!({ "targetId": tab }),
            None,
            None,
        ));
        out.push(self.call(
            "Target.attachToTarget",
            json!({ "targetId": tab, "flatten": true }),
            None,
            Some(Pending::Attached { tab: tab.clone() }),
        ));
        out.push(Out::Event(
            self.dialogs
                .get(&tab)
                .cloned()
                .unwrap_or(BrowserEvent::DialogClosed),
        ));
        self.shown = Some(Shown { tab, session: None });
        self.told = None;
        out.push(self.tabs_event());
        out.push(Out::Event(BrowserEvent::Loading { loading: false }));
        out
    }

    /// Shows the newest tab, or opens one when there's none.
    fn show_another(&mut self) -> Vec<Out> {
        if let Some(tab) = self.tabs.last() {
            let id = tab.id.clone();
            return self.show(id);
        }
        let open = self.call(
            "Target.createTarget",
            json!({ "url": "about:blank" }),
            None,
            Some(Pending::Opened),
        );
        vec![open, self.tabs_event()]
    }

    /// Lays the shown page out at the console's size and screencasts it at
    /// as many pixels as the console shows. Starting again changes the size.
    fn cast(&mut self, session: &str) -> Vec<Out> {
        let BrowserSize {
            width,
            height,
            scale,
        } = self.size;
        let metrics = json!({
            "width": width,
            "height": height,
            "deviceScaleFactor": scale,
            "mobile": false,
        });
        let cast = json!({
            "format": "jpeg",
            "quality": QUALITY,
            "maxWidth": pixels(f64::from(width) * scale),
            "maxHeight": pixels(f64::from(height) * scale),
            "everyNthFrame": 1,
        });
        vec![
            self.call(
                "Emulation.setDeviceMetricsOverride",
                metrics,
                Some(session),
                None,
            ),
            self.call("Page.startScreencast", cast, Some(session), None),
        ]
    }

    fn info(&mut self, tab: &str) -> Out {
        self.call(
            "Target.getTargetInfo",
            json!({ "targetId": tab }),
            None,
            Some(Pending::Info),
        )
    }

    fn step(&mut self, step: i64) -> Option<Out> {
        self.on_page(
            "Page.getNavigationHistory",
            json!({}),
            Some(Pending::History { step }),
        )
    }

    /// A command for the shown tab, once it's attached; until then there's
    /// nothing on screen to act on.
    fn on_page(&mut self, method: &str, params: Value, pending: Option<Pending>) -> Option<Out> {
        let session = self.session()?;
        Some(self.call(method, params, Some(&session), pending))
    }

    fn session(&self) -> Option<String> {
        self.shown.as_ref().and_then(|s| s.session.clone())
    }

    fn tabs_event(&self) -> Out {
        Out::Event(BrowserEvent::Tabs {
            tabs: self.tabs.clone(),
            shown: self.shown.as_ref().map(|s| s.tab.clone()),
        })
    }

    fn call(
        &mut self,
        method: &str,
        params: Value,
        session: Option<&str>,
        pending: Option<Pending>,
    ) -> Out {
        let id = self.next;
        self.next += 1;
        if let Some(pending) = pending
            && self.pending.len() < MAX_PENDING
        {
            self.pending.insert(id, pending);
        }
        let mut message = json!({ "id": id, "method": method });
        message["params"] = params;
        if let Some(session) = session {
            message["sessionId"] = json!(session);
        }
        Out::Devtools(message.to_string())
    }
}

#[derive(Deserialize)]
struct TargetEvent {
    #[serde(rename = "targetInfo")]
    target: Target,
}

#[derive(Deserialize)]
struct Target {
    #[serde(rename = "targetId")]
    id: String,
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    url: String,
    #[serde(rename = "openerId")]
    opener: Option<String>,
}

impl Target {
    fn tab(&self) -> BrowserTab {
        BrowserTab {
            id: self.id.clone(),
            title: self.title.clone(),
            url: self.url.clone(),
        }
    }
}

#[derive(Deserialize)]
struct Frame {
    data: String,
    metadata: FrameMetadata,
}

#[derive(Deserialize)]
struct FrameMetadata {
    #[serde(rename = "deviceWidth")]
    width: f64,
    #[serde(rename = "deviceHeight")]
    height: f64,
}

#[derive(Deserialize)]
struct Dialog {
    message: String,
    #[serde(rename = "type")]
    kind: DialogKind,
    #[serde(rename = "defaultPrompt")]
    default: Option<String>,
}

#[derive(Deserialize)]
struct History {
    #[serde(rename = "currentIndex")]
    current: i64,
    entries: Vec<HistoryEntry>,
}

#[derive(Deserialize)]
struct HistoryEntry {
    id: i64,
}

/// A size the browser can lay out and the screencast can keep up with.
fn fitted(size: BrowserSize) -> BrowserSize {
    let scale = if size.scale.is_finite() {
        size.scale.clamp(1.0, 2.0)
    } else {
        1.0
    };
    BrowserSize {
        width: size.width.clamp(200, 4096),
        height: size.height.clamp(150, 4096),
        scale,
    }
}

/// A count of pixels from arithmetic in floating point: never negative,
/// and no larger than a screen.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "clamped to what fits first"
)]
fn pixels(value: f64) -> u32 {
    value.round().clamp(0.0, 65_535.0) as u32
}

/// Where an address typed into the console goes: as typed when it has a
/// scheme, over plain HTTP when it's the workspace's own (`localhost:3000`),
/// and over HTTPS otherwise.
fn address(typed: &str) -> String {
    let typed = typed.trim();
    let has_scheme = typed.split_once(':').is_some_and(|(scheme, rest)| {
        let mut chars = scheme.chars();
        chars.next().is_some_and(|c| c.is_ascii_alphabetic())
            && chars.all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
            // `localhost:3000` is a host and a port, not a scheme.
            && !rest.chars().next().is_some_and(|c| c.is_ascii_digit())
    });
    if has_scheme || typed.is_empty() {
        return typed.to_owned();
    }
    let host = typed.split(['/', ':', '?', '#']).next().unwrap_or_default();
    let local = host == "localhost"
        || host.ends_with(".localhost")
        || host.parse::<std::net::Ipv4Addr>().is_ok();
    if local {
        format!("http://{typed}")
    } else {
        format!("https://{typed}")
    }
}

fn mask(modifiers: Modifiers) -> u8 {
    u8::from(modifiers.alt)
        | u8::from(modifiers.ctrl) << 1
        | u8::from(modifiers.meta) << 2
        | u8::from(modifiers.shift) << 3
}

fn mouse_event(mouse: &MouseInput) -> Value {
    let kind = match mouse.action {
        MouseAction::Press => "mousePressed",
        MouseAction::Release => "mouseReleased",
        MouseAction::Move => "mouseMoved",
        MouseAction::Wheel => "mouseWheel",
    };
    let button = match mouse.button {
        MouseButton::None => "none",
        MouseButton::Left => "left",
        MouseButton::Middle => "middle",
        MouseButton::Right => "right",
        MouseButton::Back => "back",
        MouseButton::Forward => "forward",
    };
    json!({
        "type": kind,
        "x": mouse.x,
        "y": mouse.y,
        "button": button,
        "buttons": mouse.buttons,
        "clickCount": mouse.clicks,
        "deltaX": mouse.delta_x,
        "deltaY": mouse.delta_y,
        "modifiers": mask(mouse.modifiers),
    })
}

/// A key as Chromium takes it: with the Windows key code its editing
/// commands go by, and the text it types, if any. A key with Control or
/// Meta is a shortcut and types nothing.
fn key_event(key: &KeyInput) -> Value {
    let shortcut = key.modifiers.ctrl || key.modifiers.meta;
    let text = match (key.action, key.key.as_str()) {
        (KeyAction::Release, _) => None,
        _ if shortcut => None,
        (KeyAction::Press, "Enter") => Some("\r"),
        (KeyAction::Press, k) if k.chars().count() == 1 => Some(k),
        (KeyAction::Press, _) => None,
    };
    let kind = match (key.action, text) {
        (KeyAction::Release, _) => "keyUp",
        (KeyAction::Press, Some(_)) => "keyDown",
        (KeyAction::Press, None) => "rawKeyDown",
    };
    let code = virtual_key(&key.code, &key.key);
    let mut event = json!({
        "type": kind,
        "key": key.key,
        "code": key.code,
        "windowsVirtualKeyCode": code,
        "nativeVirtualKeyCode": code,
        "autoRepeat": key.repeat,
        "location": key.location,
        "modifiers": mask(key.modifiers),
    });
    if let Some(text) = text {
        event["text"] = json!(text);
        event["unmodifiedText"] = json!(text);
    }
    event
}

/// The Windows virtual key code for a key, which Chromium's editing goes by:
/// without one, Backspace and the arrows do nothing. From the physical key
/// where there's one, so it doesn't depend on the layout.
fn virtual_key(code: &str, key: &str) -> u32 {
    if let Some(letter) = code.strip_prefix("Key").filter(|l| l.len() == 1) {
        return u32::from(letter.as_bytes()[0].to_ascii_uppercase());
    }
    if let Some(digit) = code.strip_prefix("Digit").filter(|d| d.len() == 1) {
        return u32::from(digit.as_bytes()[0]);
    }
    if let Some(digit) = code
        .strip_prefix("Numpad")
        .and_then(|d| d.parse::<u32>().ok())
    {
        return 0x60 + digit;
    }
    if let Some(n) = code
        .strip_prefix('F')
        .and_then(|n| n.parse::<u32>().ok())
        .filter(|n| (1..=24).contains(n))
    {
        return 0x6F + n;
    }
    match code {
        "Backspace" => 0x08,
        "Tab" => 0x09,
        "Enter" | "NumpadEnter" => 0x0D,
        "ShiftLeft" | "ShiftRight" => 0x10,
        "ControlLeft" | "ControlRight" => 0x11,
        "AltLeft" | "AltRight" => 0x12,
        "Pause" => 0x13,
        "CapsLock" => 0x14,
        "Escape" => 0x1B,
        "Space" => 0x20,
        "PageUp" => 0x21,
        "PageDown" => 0x22,
        "End" => 0x23,
        "Home" => 0x24,
        "ArrowLeft" => 0x25,
        "ArrowUp" => 0x26,
        "ArrowRight" => 0x27,
        "ArrowDown" => 0x28,
        "Insert" => 0x2D,
        "Delete" => 0x2E,
        "MetaLeft" => 0x5B,
        "MetaRight" => 0x5C,
        "ContextMenu" => 0x5D,
        "NumpadMultiply" => 0x6A,
        "NumpadAdd" => 0x6B,
        "NumpadSubtract" => 0x6D,
        "NumpadDecimal" => 0x6E,
        "NumpadDivide" => 0x6F,
        "Semicolon" => 0xBA,
        "Equal" => 0xBB,
        "Comma" => 0xBC,
        "Minus" => 0xBD,
        "Period" => 0xBE,
        "Slash" => 0xBF,
        "Backquote" => 0xC0,
        "BracketLeft" => 0xDB,
        "Backslash" => 0xDC,
        "BracketRight" => 0xDD,
        "Quote" => 0xDE,
        // A key the layout names but the code doesn't, such as from an
        // on-screen keyboard: its letter or digit, if it's one.
        _ => match key.as_bytes() {
            [c] if c.is_ascii_alphanumeric() => u32::from(c.to_ascii_uppercase()),
            _ => 0,
        },
    }
}

/// Opens the browser in front of `caller` and relays it until either side
/// leaves or their access ends.
pub async fn relay(
    app: Arc<App>,
    caller: Caller,
    ws: WorkspaceRecord,
    host: Arc<HostClient>,
    size: BrowserSize,
    console: WebSocket,
) {
    let (mut to_console, mut from_console) = console.split();
    if !crate::idle::thaw(&app, &ws, THAW).await {
        let _ = to_console
            .send(refusal("the workspace isn't running"))
            .await;
        return;
    }
    let browser = match connect(&host, ws.id).await {
        Ok(browser) => browser,
        Err(error) => {
            tracing::info!(workspace = %ws.id, %error, "couldn't open the browser");
            let _ = to_console.send(refusal(&error)).await;
            return;
        }
    };
    let (mut to_browser, mut from_browser) = browser.split();
    let (mut screen, first) = Screen::new(size);
    let mut lease = tokio::time::interval(LEASE);
    lease.tick().await;
    let mut used = Use::Idle;
    let mut revoked = false;
    let mut out = first;

    'relay: loop {
        for item in out.drain(..) {
            let sending = async {
                match item {
                    Out::Devtools(text) => to_browser.send(Devtools::text(text)).await.is_ok(),
                    Out::Event(event) => {
                        let text = serde_json::to_string(&event).unwrap_or_default();
                        to_console.send(Console::text(text)).await.is_ok()
                    }
                    Out::Image(image) => to_console.send(Console::Binary(image)).await.is_ok(),
                }
            };
            if !tokio::time::timeout(SEND, sending).await.unwrap_or(false) {
                break 'relay;
            }
        }
        tokio::select! {
            message = from_console.next() => match message {
                Some(Ok(Console::Text(text))) => {
                    match serde_json::from_str::<BrowserInput>(text.as_str()) {
                        Ok(input) => {
                            if !matches!(input, BrowserInput::Resize(_)) {
                                used = Use::Active;
                            }
                            out = screen.input(input);
                        }
                        Err(error) => tracing::debug!(%error, "ignoring browser input iglu doesn't know"),
                    }
                }
                Some(Ok(Console::Binary(_) | Console::Ping(_) | Console::Pong(_))) => {}
                Some(Ok(Console::Close(_)) | Err(_)) | None => break,
            },
            message = from_browser.next() => match message {
                Some(Ok(Devtools::Text(text))) => match serde_json::from_str::<Incoming>(text.as_str()) {
                    Ok(incoming) => out = screen.devtools(incoming),
                    Err(error) => tracing::debug!(%error, "the browser said something iglu can't read"),
                },
                Some(Ok(Devtools::Binary(_) | Devtools::Ping(_) | Devtools::Pong(_) | Devtools::Frame(_))) => {}
                Some(Ok(Devtools::Close(_)) | Err(_)) | None => break,
            },
            _ = lease.tick() => {
                // A browser in front of someone keeps its workspace awake.
                app.usage.used(ws.id);
                if !still_allowed(&app, &caller, &ws, used).await {
                    tracing::info!(workspace = %ws.id, "closing a browser whose authorization ended");
                    revoked = true;
                    break;
                }
                used = Use::Idle;
                out = screen.tick();
            }
        }
    }
    let _ = to_browser.close().await;
    if revoked {
        let _ = to_console
            .send(refusal("access to this workspace ended"))
            .await;
    } else {
        let _ = to_console.close().await;
    }
}

type Browser = WebSocketStream<reqwest::Upgraded>;

/// Connects to the workspace's browser, waiting a little for one that's
/// starting.
async fn connect(host: &HostClient, workspace: WorkspaceId) -> Result<Browser, String> {
    let port = GuestPort::try_from(BROWSER_DEBUG_PORT).map_err(|e| e.to_string())?;
    let deadline = tokio::time::Instant::now() + STARTING;
    let path = loop {
        let asked = tokio::time::timeout(VERSION_TIME, endpoint(host, workspace, port)).await;
        match asked.unwrap_or_else(|_| Err(anyhow::anyhow!("no answer"))) {
            Ok(path) => break path,
            Err(_) if tokio::time::Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            Err(error) => {
                tracing::debug!(%workspace, %error, "the browser didn't answer");
                return Err("the browser isn't running; restart its column".to_owned());
            }
        }
    };
    let stream = host
        .tunnel(workspace, port)
        .await
        .map_err(|e| e.to_string())?;
    // Chromium takes DevTools connections only for a loopback host.
    let request = format!("ws://127.0.0.1:{BROWSER_DEBUG_PORT}{path}")
        .into_client_request()
        .map_err(|e| e.to_string())?;
    // A screencast image is well under this; a message that isn't is a
    // browser iglud doesn't keep in memory for.
    let config = WebSocketConfig::default()
        .max_message_size(Some(MAX_MESSAGE))
        .max_frame_size(Some(MAX_MESSAGE));
    let (browser, _) = tokio_tungstenite::client_async_with_config(request, stream, Some(config))
        .await
        .map_err(|e| format!("the browser refused the connection: {e}"))?;
    Ok(browser)
}

/// The path of the browser's own CDP endpoint, which it makes up as it
/// starts.
async fn endpoint(
    host: &HostClient,
    workspace: WorkspaceId,
    port: GuestPort,
) -> anyhow::Result<String> {
    let stream = host.tunnel(workspace, port).await?;
    let (mut sender, connection) =
        hyper::client::conn::http1::handshake(TokioIo::new(stream)).await?;
    tokio::spawn(connection);
    let request = hyper::Request::get("/json/version")
        .header(
            hyper::header::HOST,
            format!("127.0.0.1:{BROWSER_DEBUG_PORT}"),
        )
        .body(Empty::<Bytes>::new())?;
    let response = sender.send_request(request).await?;
    anyhow::ensure!(response.status().is_success(), "{}", response.status());
    // What's on the port is the workspace's: a version report is small.
    let body = Limited::new(response.into_body(), VERSION_BYTES)
        .collect()
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?
        .to_bytes();
    endpoint_path(&body)
}

fn endpoint_path(version: &[u8]) -> anyhow::Result<String> {
    #[derive(Deserialize)]
    struct Version {
        #[serde(rename = "webSocketDebuggerUrl")]
        url: String,
    }
    let version: Version = serde_json::from_slice(version)?;
    let url = url::Url::parse(&version.url)?;
    Ok(url.path().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: BrowserSize = BrowserSize {
        width: 800,
        height: 600,
        scale: 2.0,
    };

    fn incoming(value: Value) -> Incoming {
        serde_json::from_value(value).expect("a message from the browser")
    }

    fn sent(out: &[Out]) -> Vec<Value> {
        out.iter()
            .filter_map(|o| match o {
                Out::Devtools(text) => Some(serde_json::from_str(text).expect("JSON")),
                Out::Event(_) | Out::Image(_) => None,
            })
            .collect()
    }

    fn methods(out: &[Out]) -> Vec<String> {
        sent(out)
            .iter()
            .map(|m| m["method"].as_str().expect("a method").to_owned())
            .collect()
    }

    fn page(id: &str, opener: Option<&str>) -> Incoming {
        incoming(json!({
            "method": "Target.targetCreated",
            "params": { "targetInfo": { "targetId": id, "type": "page", "title": "", "url": "about:blank", "openerId": opener } },
        }))
    }

    /// A screen showing tab `t1`, attached as session `s1`.
    fn showing() -> Screen {
        let (mut screen, _) = Screen::new(SIZE);
        screen.devtools(page("t1", None));
        let out = screen.devtools(incoming(json!({ "id": 1, "result": {} })));
        let attach = sent(&out)
            .into_iter()
            .find(|m| m["method"] == "Target.attachToTarget")
            .expect("attaching");
        screen.devtools(incoming(
            json!({ "id": attach["id"], "result": { "sessionId": "s1" } }),
        ));
        screen
    }

    #[test]
    fn opening_shows_a_tab_at_the_consoles_size() {
        let (mut screen, first) = Screen::new(SIZE);
        assert_eq!(methods(&first), ["Target.setDiscoverTargets"]);
        // A tab that's there is reported before discovery is answered.
        let out = screen.devtools(page("t1", None));
        assert!(methods(&out).is_empty());
        let out = screen.devtools(incoming(json!({ "id": 1, "result": {} })));
        assert_eq!(
            methods(&out),
            ["Target.activateTarget", "Target.attachToTarget"]
        );
        let attach = sent(&out).remove(1);
        let out = screen.devtools(incoming(
            json!({ "id": attach["id"], "result": { "sessionId": "s1" } }),
        ));
        let cast = sent(&out);
        assert_eq!(
            methods(&out),
            [
                "Page.enable",
                "Page.addScriptToEvaluateOnNewDocument",
                "Runtime.evaluate",
                "Emulation.setDeviceMetricsOverride",
                "Page.startScreencast"
            ]
        );
        assert!(cast.iter().all(|m| m["sessionId"] == "s1"));
        assert_eq!(cast[3]["params"]["width"], 800);
        assert_eq!(cast[4]["params"]["maxWidth"], 1600);
    }

    #[test]
    fn a_browser_without_tabs_gets_one() {
        let (mut screen, _) = Screen::new(SIZE);
        let out = screen.devtools(incoming(json!({ "id": 1, "result": {} })));
        assert_eq!(methods(&out), ["Target.createTarget"]);
        let create = sent(&out).remove(0);
        let out = screen.devtools(incoming(
            json!({ "id": create["id"], "result": { "targetId": "t9" } }),
        ));
        assert!(methods(&out).contains(&"Target.attachToTarget".to_owned()));
    }

    #[test]
    fn frames_of_the_shown_tab_reach_the_console_and_every_frame_is_acknowledged() {
        let mut screen = showing();
        let frame = |session: &str| {
            incoming(json!({
                "method": "Page.screencastFrame",
                "sessionId": session,
                "params": { "data": "aGk=", "sessionId": 7, "metadata": { "deviceWidth": 800.0, "deviceHeight": 600.0 } },
            }))
        };
        let out = screen.devtools(frame("s1"));
        assert_eq!(sent(&out)[0]["method"], "Page.screencastFrameAck");
        assert_eq!(sent(&out)[0]["params"]["sessionId"], 7);
        assert!(out.contains(&Out::Event(BrowserEvent::Viewport {
            width: 800,
            height: 600
        })));
        assert!(out.contains(&Out::Image(Bytes::from_static(b"hi"))));
        // The size is told once, until it changes.
        let out = screen.devtools(frame("s1"));
        assert!(!out.iter().any(|o| matches!(o, Out::Event(_))));
        // A tab left behind may still send a frame.
        let out = screen.devtools(frame("s0"));
        assert_eq!(methods(&out), ["Page.screencastFrameAck"]);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn a_tab_the_shown_page_opens_comes_to_the_front_and_closing_it_goes_back() {
        let mut screen = showing();
        let out = screen.devtools(page("t2", Some("t1")));
        let calls = sent(&out);
        assert_eq!(calls[0]["method"], "Target.detachFromTarget");
        assert_eq!(calls[0]["params"]["sessionId"], "s1");
        assert!(
            calls
                .iter()
                .any(|m| m["method"] == "Target.attachToTarget" && m["params"]["targetId"] == "t2")
        );
        // A tab opened some other way, as by an agent, stays behind.
        let out = screen.devtools(page("t3", None));
        assert!(methods(&out).is_empty());
        let out = screen.devtools(incoming(
            json!({ "method": "Target.targetDestroyed", "params": { "targetId": "t2" } }),
        ));
        assert!(
            sent(&out)
                .iter()
                .any(|m| m["method"] == "Target.attachToTarget" && m["params"]["targetId"] == "t3")
        );
    }

    #[test]
    fn a_page_that_stops_loading_has_its_title_read_again() {
        let mut screen = showing();
        let stopped = incoming(
            json!({ "method": "Page.frameStoppedLoading", "sessionId": "s1", "params": { "frameId": "t1" } }),
        );
        let out = screen.devtools(stopped);
        assert!(out.contains(&Out::Event(BrowserEvent::Loading { loading: false })));
        let ask = sent(&out).remove(0);
        assert_eq!(ask["method"], "Target.getTargetInfo");
        let out = screen.devtools(incoming(json!({
            "id": ask["id"],
            "result": { "targetInfo": { "targetId": "t1", "type": "page", "title": "Docs", "url": "http://localhost:3000/" } },
        })));
        let Some(Out::Event(BrowserEvent::Tabs { tabs, .. })) = out.first() else {
            panic!("{out:?}");
        };
        assert_eq!(tabs[0].title, "Docs");
        // A frame inside the page loading is no news.
        let inner = incoming(
            json!({ "method": "Page.frameStoppedLoading", "sessionId": "s1", "params": { "frameId": "f2" } }),
        );
        assert!(screen.devtools(inner).is_empty());
    }

    fn answer(screen: &mut Screen, call: &Value, result: Value) -> Vec<Out> {
        let mut message = json!({ "id": call["id"] });
        message["result"] = result;
        screen.devtools(incoming(message))
    }

    fn attaches(out: &[Out], tab: &str) -> Option<Value> {
        sent(out)
            .into_iter()
            .find(|m| m["method"] == "Target.attachToTarget" && m["params"]["targetId"] == tab)
    }

    #[test]
    fn switching_back_before_an_attach_answers_keeps_one_session() {
        let mut screen = showing();
        screen.devtools(page("t2", None));
        let to_b = screen.input(BrowserInput::Show { tab: "t2".into() });
        let to_a = screen.input(BrowserInput::Show { tab: "t1".into() });
        let first = attaches(&to_a, "t1").expect("attaching to t1");
        answer(&mut screen, &first, json!({ "sessionId": "s3" }));
        // The answer for t2 comes late, for a tab no longer shown.
        let late = attaches(&to_b, "t2").expect("attaching to t2");
        let out = answer(&mut screen, &late, json!({ "sessionId": "s2" }));
        assert_eq!(sent(&out)[0]["method"], "Target.detachFromTarget");
        assert_eq!(sent(&out)[0]["params"]["sessionId"], "s2");
        // So does a second answer for t1, which is attached already.
        let again = screen.input(BrowserInput::Show { tab: "t2".into() });
        let back = screen.input(BrowserInput::Show { tab: "t1".into() });
        let one = attaches(&back, "t1").expect("attaching");
        answer(&mut screen, &one, json!({ "sessionId": "s4" }));
        let b = attaches(&again, "t2").expect("attaching");
        let out = answer(&mut screen, &b, json!({ "sessionId": "s5" }));
        assert_eq!(sent(&out)[0]["params"]["sessionId"], "s5");
        assert_eq!(screen.session().as_deref(), Some("s4"));
    }

    #[test]
    fn choosing_a_tab_that_just_closed_leaves_the_shown_one() {
        let mut screen = showing();
        let out = screen.input(BrowserInput::Show { tab: "gone".into() });
        assert!(sent(&out).is_empty());
        assert_eq!(screen.session().as_deref(), Some("s1"));
    }

    #[test]
    fn a_tab_that_goes_as_it_is_attached_to_gives_way_to_another() {
        let mut screen = showing();
        screen.devtools(page("t2", None));
        let out = screen.input(BrowserInput::Show { tab: "t2".into() });
        let attach = attaches(&out, "t2").expect("attaching");
        let out = screen.devtools(incoming(
            json!({ "id": attach["id"], "error": { "message": "No target with given id" } }),
        ));
        assert!(
            attaches(&out, "t2").is_some() || attaches(&out, "t1").is_some(),
            "{out:?}"
        );
    }

    #[test]
    fn a_dialog_goes_with_its_tab_and_comes_back_with_it() {
        let mut screen = showing();
        screen.devtools(page("t2", None));
        let opened = incoming(json!({
            "method": "Page.javascriptDialogOpening",
            "sessionId": "s1",
            "params": { "message": "Sure?", "type": "confirm", "defaultPrompt": "" },
        }));
        assert!(matches!(
            screen.devtools(opened).first(),
            Some(Out::Event(BrowserEvent::Dialog { .. }))
        ));
        let away = screen.input(BrowserInput::Show { tab: "t2".into() });
        assert!(away.contains(&Out::Event(BrowserEvent::DialogClosed)));
        let back = screen.input(BrowserInput::Show { tab: "t1".into() });
        assert!(back.iter().any(
            |o| matches!(o, Out::Event(BrowserEvent::Dialog { message, .. }) if message == "Sure?")
        ));
    }

    #[test]
    fn an_answer_to_a_dialog_already_gone_puts_it_away() {
        let mut screen = showing();
        let out = screen.input(BrowserInput::Answer {
            accept: true,
            text: None,
        });
        let call = sent(&out).remove(0);
        let out = screen.devtools(incoming(
            json!({ "id": call["id"], "error": { "message": "No dialog is showing" } }),
        ));
        assert_eq!(out, vec![Out::Event(BrowserEvent::DialogClosed)]);
    }

    #[test]
    fn a_browser_that_never_answers_doesnt_grow_what_waits_for_one() {
        let mut screen = showing();
        let moved = || {
            incoming(
                json!({ "method": "Page.navigatedWithinDocument", "sessionId": "s1", "params": {} }),
            )
        };
        for _ in 0..MAX_PENDING * 2 {
            screen.devtools(moved());
        }
        assert!(screen.pending.len() <= MAX_PENDING);
        for n in 0..MAX_TABS * 2 {
            screen.devtools(page(&format!("x{n}"), None));
        }
        assert!(screen.tabs.len() <= MAX_TABS);
    }

    #[test]
    fn a_title_the_page_sets_itself_is_read_as_time_passes() {
        let (mut screen, _) = Screen::new(SIZE);
        assert!(screen.tick().is_empty());
        let mut screen = showing();
        let ask = sent(&screen.tick()).remove(0);
        assert_eq!(ask["method"], "Target.getTargetInfo");
        assert_eq!(ask["params"]["targetId"], "t1");
    }

    #[test]
    fn input_waits_for_a_tab_to_act_on() {
        let (mut screen, _) = Screen::new(SIZE);
        assert!(screen.input(BrowserInput::Reload).is_empty());
        let mut screen = showing();
        let out = screen.input(BrowserInput::Reload);
        assert_eq!(sent(&out)[0]["sessionId"], "s1");
    }

    #[test]
    fn back_goes_to_the_entry_before_the_current_one() {
        let mut screen = showing();
        let out = screen.input(BrowserInput::Back);
        let ask = sent(&out).remove(0);
        let out = screen.devtools(incoming(json!({
            "id": ask["id"],
            "result": { "currentIndex": 1, "entries": [{ "id": 10 }, { "id": 11 }] },
        })));
        assert_eq!(sent(&out)[0]["params"]["entryId"], 10);
        // There's nothing before the first.
        let out = screen.input(BrowserInput::Back);
        let ask = sent(&out).remove(0);
        let out = screen.devtools(incoming(json!({
            "id": ask["id"],
            "result": { "currentIndex": 0, "entries": [{ "id": 10 }] },
        })));
        assert!(out.is_empty());
    }

    #[test]
    fn addresses_go_where_a_browser_would_take_them() {
        assert_eq!(address("localhost:3000"), "http://localhost:3000");
        assert_eq!(address("app.localhost/x"), "http://app.localhost/x");
        assert_eq!(address("127.0.0.1:8080/a?b"), "http://127.0.0.1:8080/a?b");
        assert_eq!(address("example.com"), "https://example.com");
        assert_eq!(address(" https://example.com/a "), "https://example.com/a");
        assert_eq!(address("about:blank"), "about:blank");
        assert_eq!(
            address("file:///home/dev/index.html"),
            "file:///home/dev/index.html"
        );
    }

    fn key(action: KeyAction, key: &str, code: &str, modifiers: Modifiers) -> Value {
        key_event(&KeyInput {
            action,
            key: key.to_owned(),
            code: code.to_owned(),
            repeat: false,
            location: 0,
            modifiers,
        })
    }

    #[test]
    fn keys_type_their_text_and_shortcuts_type_nothing() {
        let plain = Modifiers::default();
        let a = key(KeyAction::Press, "a", "KeyA", plain);
        assert_eq!(
            (a["type"].as_str(), a["text"].as_str()),
            (Some("keyDown"), Some("a"))
        );
        assert_eq!(a["windowsVirtualKeyCode"], 65);
        let enter = key(KeyAction::Press, "Enter", "Enter", plain);
        assert_eq!(enter["text"], "\r");
        let back = key(KeyAction::Press, "Backspace", "Backspace", plain);
        assert_eq!(
            (
                back["type"].as_str(),
                back["windowsVirtualKeyCode"].as_u64()
            ),
            (Some("rawKeyDown"), Some(8))
        );
        assert!(back.get("text").is_none());
        let ctrl = Modifiers {
            ctrl: true,
            ..plain
        };
        let select_all = key(KeyAction::Press, "a", "KeyA", ctrl);
        assert_eq!(select_all["type"], "rawKeyDown");
        assert_eq!(select_all["modifiers"], 2);
        let up = key(KeyAction::Release, "a", "KeyA", plain);
        assert_eq!(up["type"], "keyUp");
        assert!(up.get("text").is_none());
    }

    #[test]
    fn virtual_keys_follow_the_physical_key() {
        assert_eq!(virtual_key("KeyQ", "a"), 0x51);
        assert_eq!(virtual_key("Digit7", "&"), 0x37);
        assert_eq!(virtual_key("ArrowLeft", "ArrowLeft"), 0x25);
        assert_eq!(virtual_key("F5", "F5"), 0x74);
        assert_eq!(virtual_key("Numpad3", "3"), 0x63);
        assert_eq!(virtual_key("", "x"), 0x58);
        assert_eq!(virtual_key("", "é"), 0);
    }

    #[test]
    fn sizes_stay_within_what_the_browser_can_show() {
        let huge = fitted(BrowserSize {
            width: 99_999,
            height: 1,
            scale: f64::NAN,
        });
        assert_eq!((huge.width, huge.height), (4096, 150));
        assert!((huge.scale - 1.0).abs() < f64::EPSILON);
        let sharp = fitted(BrowserSize {
            width: 800,
            height: 600,
            scale: 3.0,
        });
        assert!((sharp.scale - 2.0).abs() < f64::EPSILON);
    }

    #[test]
    fn the_endpoint_is_the_path_chromium_reports() {
        let version = br#"{"Browser":"Chrome/140","webSocketDebuggerUrl":"ws://127.0.0.1:9222/devtools/browser/abc-123"}"#;
        assert_eq!(
            endpoint_path(version).expect("a path"),
            "/devtools/browser/abc-123"
        );
    }
}
