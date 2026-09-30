//! Which cross-origin requests the preview gateway forwards, and which
//! requests the console accepts.
//!
//! Previews and the console usually share a registrable domain, so browsers
//! treat them as one *site*: `SameSite` cookies don't separate them, and a
//! preview can make the browser send credentialed requests to other previews
//! and to the console. These rules close that gap using Fetch Metadata and
//! `Origin`, which browsers set and pages can't forge.

use std::str::FromStr;

use crate::ParseError;

/// The `Sec-Fetch-Site` request header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FetchSite {
    SameOrigin,
    SameSite,
    CrossSite,
    /// A user-initiated navigation, such as typing the URL or opening a bookmark.
    None,
}

impl FromStr for FetchSite {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "same-origin" => Ok(Self::SameOrigin),
            "same-site" => Ok(Self::SameSite),
            "cross-site" => Ok(Self::CrossSite),
            "none" => Ok(Self::None),
            _ => Err(ParseError::new("Sec-Fetch-Site", "unknown value")),
        }
    }
}

/// The `Sec-Fetch-Mode` request header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FetchMode {
    Navigate,
    Cors,
    NoCors,
    SameOrigin,
    WebSocket,
}

impl FromStr for FetchMode {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "navigate" => Ok(Self::Navigate),
            "cors" => Ok(Self::Cors),
            "no-cors" => Ok(Self::NoCors),
            "same-origin" => Ok(Self::SameOrigin),
            "websocket" => Ok(Self::WebSocket),
            _ => Err(ParseError::new("Sec-Fetch-Mode", "unknown value")),
        }
    }
}

/// Whether a request method can change state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MethodClass {
    /// GET, HEAD, OPTIONS.
    Safe,
    Unsafe,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport {
    Http(MethodClass),
    WebSocket,
}

/// Where a preview request came from, relative to the route it targets. The
/// shell derives this from `Origin`, falling back to `Referer`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestSource {
    /// No `Origin` or `Referer`: not a credentialed cross-origin browser request.
    Unknown,
    SameOrigin,
    /// Another route of the same workspace.
    SameWorkspace,
    Elsewhere,
}

/// Browser-supplied metadata about a request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FetchMetadata {
    pub site: Option<FetchSite>,
    pub mode: Option<FetchMode>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewAccess {
    Forward,
    Reject,
}

/// Whether the gateway forwards a request to a preview route.
///
/// A workspace's routes may call each other, so a frontend can talk to its
/// API. Every other cross-origin request is rejected, except top-level
/// navigations, so following a link between apps still works.
#[must_use]
pub fn preview_access(
    transport: Transport,
    fetch: FetchMetadata,
    source: RequestSource,
) -> PreviewAccess {
    let trusted_source = match source {
        RequestSource::SameOrigin | RequestSource::SameWorkspace => true,
        RequestSource::Unknown | RequestSource::Elsewhere => false,
    };
    match transport {
        // Browsers always send Origin on WebSocket handshakes, and WebSockets
        // have no CORS, so the Origin decides alone.
        Transport::WebSocket => match source {
            RequestSource::Unknown | RequestSource::SameOrigin | RequestSource::SameWorkspace => {
                PreviewAccess::Forward
            }
            RequestSource::Elsewhere => PreviewAccess::Reject,
        },
        Transport::Http(method) => match fetch.site {
            // Without Fetch Metadata the request isn't from a modern browser
            // context that could carry the user's cookies cross-site.
            None | Some(FetchSite::SameOrigin | FetchSite::None) => PreviewAccess::Forward,
            Some(FetchSite::SameSite | FetchSite::CrossSite) => {
                let top_level_navigation =
                    fetch.mode == Some(FetchMode::Navigate) && method == MethodClass::Safe;
                if top_level_navigation || trusted_source {
                    PreviewAccess::Forward
                } else {
                    PreviewAccess::Reject
                }
            }
        },
    }
}

/// How a console request authenticated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Credential {
    /// A browser session cookie, which the browser attaches automatically.
    Cookie,
    /// A bearer token, which a page can't make the browser attach.
    Bearer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConsoleOrigin {
    Absent,
    Exact,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CsrfToken {
    Valid,
    MissingOrWrong,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ConsoleRejection {
    #[error("cross-origin WebSocket")]
    CrossOriginWebSocket,
    #[error("state-changing request from another origin")]
    CrossOriginMutation,
    #[error("missing or wrong CSRF token")]
    Csrf,
}

/// Whether the console accepts a request. Without these checks, a preview
/// page could make the browser open a terminal WebSocket to the console with
/// the user's cookie and get a shell in any workspace.
///
/// # Errors
///
/// Why the console refuses the request.
pub fn console_access(
    credential: Credential,
    transport: Transport,
    fetch: FetchMetadata,
    origin: ConsoleOrigin,
    csrf: CsrfToken,
) -> Result<(), ConsoleRejection> {
    match credential {
        Credential::Bearer => return Ok(()),
        Credential::Cookie => {}
    }
    match transport {
        Transport::WebSocket => match origin {
            ConsoleOrigin::Exact => Ok(()),
            ConsoleOrigin::Absent | ConsoleOrigin::Other => {
                Err(ConsoleRejection::CrossOriginWebSocket)
            }
        },
        Transport::Http(MethodClass::Safe) => Ok(()),
        Transport::Http(MethodClass::Unsafe) => {
            match (fetch.site, origin) {
                (Some(FetchSite::SameOrigin), ConsoleOrigin::Exact | ConsoleOrigin::Absent) => {}
                (Some(FetchSite::SameOrigin), ConsoleOrigin::Other)
                | (None | Some(FetchSite::SameSite | FetchSite::CrossSite | FetchSite::None), _) => {
                    return Err(ConsoleRejection::CrossOriginMutation);
                }
            }
            match csrf {
                CsrfToken::Valid => Ok(()),
                CsrfToken::MissingOrWrong => Err(ConsoleRejection::Csrf),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GET: Transport = Transport::Http(MethodClass::Safe);
    const POST: Transport = Transport::Http(MethodClass::Unsafe);

    fn meta(site: Option<FetchSite>, mode: Option<FetchMode>) -> FetchMetadata {
        FetchMetadata { site, mode }
    }

    #[test]
    fn same_origin_and_user_initiated_requests_are_forwarded() {
        for site in [Some(FetchSite::SameOrigin), Some(FetchSite::None), None] {
            for transport in [GET, POST] {
                assert_eq!(
                    preview_access(
                        transport,
                        meta(site, Some(FetchMode::Cors)),
                        RequestSource::Unknown
                    ),
                    PreviewAccess::Forward
                );
            }
        }
    }

    #[test]
    fn a_workspace_can_call_its_own_routes() {
        for transport in [GET, POST, Transport::WebSocket] {
            assert_eq!(
                preview_access(
                    transport,
                    meta(Some(FetchSite::SameSite), Some(FetchMode::Cors)),
                    RequestSource::SameWorkspace
                ),
                PreviewAccess::Forward
            );
        }
    }

    #[test]
    fn other_workspaces_cannot_call_a_route() {
        for site in [FetchSite::SameSite, FetchSite::CrossSite] {
            for mode in [FetchMode::Cors, FetchMode::NoCors] {
                for transport in [GET, POST] {
                    assert_eq!(
                        preview_access(
                            transport,
                            meta(Some(site), Some(mode)),
                            RequestSource::Elsewhere
                        ),
                        PreviewAccess::Reject
                    );
                }
            }
        }
        assert_eq!(
            preview_access(
                Transport::WebSocket,
                meta(None, None),
                RequestSource::Elsewhere
            ),
            PreviewAccess::Reject
        );
    }

    #[test]
    fn cross_site_subresources_without_an_origin_are_rejected() {
        assert_eq!(
            preview_access(
                GET,
                meta(Some(FetchSite::SameSite), Some(FetchMode::NoCors)),
                RequestSource::Unknown
            ),
            PreviewAccess::Reject
        );
    }

    #[test]
    fn links_between_apps_still_work() {
        assert_eq!(
            preview_access(
                GET,
                meta(Some(FetchSite::SameSite), Some(FetchMode::Navigate)),
                RequestSource::Elsewhere
            ),
            PreviewAccess::Forward
        );
        assert_eq!(
            preview_access(
                POST,
                meta(Some(FetchSite::SameSite), Some(FetchMode::Navigate)),
                RequestSource::Elsewhere
            ),
            PreviewAccess::Reject
        );
    }

    const SAME: FetchMetadata = FetchMetadata {
        site: Some(FetchSite::SameOrigin),
        mode: Some(FetchMode::Cors),
    };

    #[test]
    fn console_websockets_need_the_exact_origin() {
        assert_eq!(
            console_access(
                Credential::Cookie,
                Transport::WebSocket,
                SAME,
                ConsoleOrigin::Exact,
                CsrfToken::MissingOrWrong
            ),
            Ok(())
        );
        for origin in [ConsoleOrigin::Absent, ConsoleOrigin::Other] {
            assert_eq!(
                console_access(
                    Credential::Cookie,
                    Transport::WebSocket,
                    SAME,
                    origin,
                    CsrfToken::Valid
                ),
                Err(ConsoleRejection::CrossOriginWebSocket)
            );
        }
    }

    #[test]
    fn console_mutations_need_same_origin_and_csrf() {
        assert_eq!(
            console_access(
                Credential::Cookie,
                POST,
                SAME,
                ConsoleOrigin::Exact,
                CsrfToken::Valid
            ),
            Ok(())
        );
        assert_eq!(
            console_access(
                Credential::Cookie,
                POST,
                SAME,
                ConsoleOrigin::Exact,
                CsrfToken::MissingOrWrong
            ),
            Err(ConsoleRejection::Csrf)
        );
        for site in [
            None,
            Some(FetchSite::SameSite),
            Some(FetchSite::CrossSite),
            Some(FetchSite::None),
        ] {
            assert_eq!(
                console_access(
                    Credential::Cookie,
                    POST,
                    meta(site, None),
                    ConsoleOrigin::Exact,
                    CsrfToken::Valid
                ),
                Err(ConsoleRejection::CrossOriginMutation)
            );
        }
        assert_eq!(
            console_access(
                Credential::Cookie,
                POST,
                SAME,
                ConsoleOrigin::Other,
                CsrfToken::Valid
            ),
            Err(ConsoleRejection::CrossOriginMutation)
        );
    }

    #[test]
    fn bearer_requests_are_not_csrf_able() {
        assert_eq!(
            console_access(
                Credential::Bearer,
                POST,
                meta(None, None),
                ConsoleOrigin::Absent,
                CsrfToken::MissingOrWrong
            ),
            Ok(())
        );
    }
}
