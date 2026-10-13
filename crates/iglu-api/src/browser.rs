//! The workspace's browser as the console shows it, over
//! `GET /v1/workspaces/{id}/browser?scale=<n>`: its display, over VNC (RFB)
//! in binary frames both ways, and iglu's own messages in text frames,
//! [`BrowserEvent`]s to the console and [`BrowserInput`]s from it. The
//! first frame is [`BrowserEvent::Display`], before any of VNC's.
//!
//! The browser draws itself, its tabs, menus, dialogs and pickers
//! included, so the console needn't know them.

use iglu_domain::display::DisplayScale;
use iglu_domain::download::DownloadName;
use serde::{Deserialize, Serialize};

/// The pixel ratio of the screen the console shows the browser on, which
/// the browser is drawn at.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct BrowserQuery {
    pub scale: DisplayScale,
}

/// What the console asks of the browser, beside what VNC carries.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, tag = "type"))]
pub enum BrowserInput {
    /// The screen it's shown on now has this pixel ratio.
    Scale { scale: DisplayScale },
    /// Opens a new tab at an address as typed: a URL, or a host and maybe a
    /// port.
    Open { address: String },
}

/// What iglu tells the console of the browser, beside what VNC carries.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, tag = "type"))]
pub enum BrowserEvent {
    /// What the display draws and encodes with: on the host's GPU, which
    /// sends H.264, or on the CPU, which sends losslessly.
    Display { hardware: bool },
    /// The browser finished downloading a file, to `path` in the workspace:
    /// by `name`, from GET `/v1/workspaces/{id}/downloads/{name}`, when
    /// it's in the user's Downloads, where it's saved from.
    Downloaded {
        name: Option<DownloadName>,
        path: String,
    },
}
