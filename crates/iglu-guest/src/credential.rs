//! A Git credential helper that answers from the delivered secrets.
//!
//! Git runs `iglu-guest git-credential get` and writes `key=value` lines;
//! the answer is `username` and `password` for a matching HTTPS host.

use iglu_domain::repo::GitHost;
use serde::{Deserialize, Serialize};

/// One stored credential, as `install-secrets` writes it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stored {
    pub host: GitHost,
    pub username: String,
    pub password: String,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Request {
    protocol: Option<String>,
    host: Option<String>,
}

/// Parses Git's credential request. Unknown keys are ignored, as the
/// protocol requires.
#[must_use]
pub fn parse_request(input: &str) -> Request {
    let mut request = Request {
        protocol: None,
        host: None,
    };
    for line in input.lines().take_while(|line| !line.is_empty()) {
        if let Some((key, value)) = line.split_once('=') {
            match key {
                "protocol" => request.protocol = Some(value.to_owned()),
                "host" => request.host = Some(value.to_ascii_lowercase()),
                _ => {}
            }
        }
    }
    request
}

/// The helper's answer, or `None` to let Git try other helpers.
#[must_use]
pub fn answer(request: &Request, stored: &[Stored]) -> Option<String> {
    if request.protocol.as_deref() != Some("https") {
        return None;
    }
    let host = request.host.as_deref()?;
    let found = stored
        .iter()
        .find(|credential| credential.host.as_str() == host)?;
    let safe = |value: &str| !value.contains('\n') && !value.contains('\0');
    if !safe(&found.username) || !safe(&found.password) {
        return None;
    }
    Some(format!(
        "username={}\npassword={}\n",
        found.username, found.password
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stored() -> Vec<Stored> {
        vec![Stored {
            host: "github.com".parse().expect("valid host"),
            username: "x-access-token".into(),
            password: "tok".into(),
        }]
    }

    #[test]
    fn answers_matching_https_hosts() {
        let request = parse_request("protocol=https\nhost=GitHub.com\npath=acme/app.git\n\n");
        assert_eq!(
            answer(&request, &stored()).as_deref(),
            Some("username=x-access-token\npassword=tok\n")
        );
    }

    #[test]
    fn stays_quiet_otherwise() {
        assert_eq!(
            answer(
                &parse_request("protocol=http\nhost=github.com\n"),
                &stored()
            ),
            None
        );
        assert_eq!(
            answer(
                &parse_request("protocol=https\nhost=gitlab.com\n"),
                &stored()
            ),
            None
        );
        assert_eq!(answer(&parse_request("protocol=https\n"), &stored()), None);
    }
}
