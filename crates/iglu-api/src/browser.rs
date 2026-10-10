//! The workspace's browser as the console shows it, over
//! `GET /v1/workspaces/{id}/browser`: the console sends what the person does
//! as [`BrowserInput`] text frames, and gets [`BrowserEvent`] text frames and
//! the screen as binary frames, each a JPEG image of the shown tab.
//!
//! iglud speaks Chrome's debugging protocol (CDP) to the browser, so the
//! console needn't.

use serde::{Deserialize, Serialize};

/// The size the console asks the page to lay out at when it opens the
/// browser: CSS pixels, and device pixels per CSS pixel.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct BrowserSize {
    pub width: u32,
    pub height: u32,
    pub scale: f64,
}

/// What the person does to the browser.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, tag = "type", rename_all = "snake_case")
)]
pub enum BrowserInput {
    /// The space it's shown in changed size.
    Resize(BrowserSize),
    Mouse(MouseInput),
    Key(KeyInput),
    /// Text for wherever the page's caret is: a paste, or what an input
    /// method composed.
    Text {
        text: String,
    },
    /// Asks for what's selected in the page, which comes back as
    /// [`BrowserEvent::Copied`].
    Copy,
    /// Goes to an address as typed: a URL, or a host and maybe a port.
    Navigate {
        address: String,
    },
    Back,
    Forward,
    Reload,
    /// Shows another of the browser's tabs.
    Show {
        tab: String,
    },
    /// Opens a new, blank tab and shows it.
    Open,
    Close {
        tab: String,
    },
    /// Answers the dialog the shown page opened.
    Answer {
        accept: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        text: Option<String>,
    },
}

/// A mouse button pressed, released, moved or wheeled over the page, at a
/// point in CSS pixels from the page's top left.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct MouseInput {
    pub action: MouseAction,
    pub x: f64,
    pub y: f64,
    /// The button that changed; `none` for a move or the wheel.
    pub button: MouseButton,
    /// The buttons held, as `MouseEvent.buttons` gives them.
    pub buttons: u8,
    /// How many presses in a row this is, for double and triple clicks.
    pub clicks: u8,
    /// How far the wheel turned, in CSS pixels.
    pub delta_x: f64,
    pub delta_y: f64,
    pub modifiers: Modifiers,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, rename_all = "snake_case")
)]
pub enum MouseAction {
    Press,
    Release,
    Move,
    Wheel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, rename_all = "snake_case")
)]
pub enum MouseButton {
    None,
    Left,
    Middle,
    Right,
    Back,
    Forward,
}

/// A key pressed or released, as a `KeyboardEvent` names it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct KeyInput {
    pub action: KeyAction,
    /// What the key means, such as `a`, `A` or `Enter`.
    pub key: String,
    /// Which key it is, such as `KeyA` or `Enter`.
    pub code: String,
    /// Whether it's held down and repeating.
    pub repeat: bool,
    /// Where on the keyboard, for keys there are two of: `KeyboardEvent.location`.
    pub location: u8,
    pub modifiers: Modifiers,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, rename_all = "snake_case")
)]
pub enum KeyAction {
    Press,
    Release,
}

/// The modifier keys held. The browser runs on Linux, so the console sends
/// a Mac's Command as Control, which is what its shortcuts use there.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each modifier is held or not, independently, as the browser reports it"
)]
pub struct Modifiers {
    pub alt: bool,
    pub ctrl: bool,
    pub meta: bool,
    pub shift: bool,
}

/// What iglud tells the console about the browser.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, tag = "type", rename_all = "snake_case")
)]
pub enum BrowserEvent {
    /// The browser's tabs in the order they opened, and the one shown, once
    /// there is one.
    Tabs {
        tabs: Vec<BrowserTab>,
        shown: Option<String>,
    },
    /// The size of the page the images from here on show, in CSS pixels.
    Viewport { width: u32, height: u32 },
    /// Whether the shown tab is loading a page.
    Loading { loading: bool },
    /// The shown page opened a dialog, and waits for an answer.
    Dialog {
        kind: DialogKind,
        message: String,
        /// What a prompt suggests as the answer.
        default: String,
    },
    /// The dialog was answered, here or by something else driving the browser.
    DialogClosed,
    /// What was selected in the page, for the clipboard.
    Copied { text: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct BrowserTab {
    pub id: String,
    pub title: String,
    pub url: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, rename_all = "snake_case")
)]
pub enum DialogKind {
    Alert,
    Confirm,
    Prompt,
    /// Asks whether to leave the page.
    Beforeunload,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_reads_as_the_console_writes_it() {
        let input: BrowserInput = serde_json::from_str(
            r#"{"type":"key","action":"press","key":"a","code":"KeyA","repeat":false,"location":0,"modifiers":{"alt":false,"ctrl":true,"meta":false,"shift":false}}"#,
        )
        .expect("input");
        assert!(matches!(input, BrowserInput::Key(KeyInput { ref key, .. }) if key == "a"));
        let input: BrowserInput = serde_json::from_str(r#"{"type":"back"}"#).expect("input");
        assert_eq!(input, BrowserInput::Back);
        let input: BrowserInput =
            serde_json::from_str(r#"{"type":"resize","width":800,"height":600,"scale":2}"#)
                .expect("input");
        assert_eq!(
            input,
            BrowserInput::Resize(BrowserSize {
                width: 800,
                height: 600,
                scale: 2.0
            })
        );
    }
}
