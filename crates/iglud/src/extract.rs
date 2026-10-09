//! Request extractors that refuse with an [`ApiError`], so a request that
//! doesn't parse is answered like any other bad request: as an `ErrorBody`
//! naming the input, not axum's plain-text rejection. iglud's handlers use
//! these instead of axum's own, which clippy enforces (`clippy.toml`).

use axum::extract::path::ErrorKind as PathErrorKind;
use axum::extract::rejection::PathRejection;
use axum::extract::{FromRequest, FromRequestParts, RawPathParams, Request};
use axum::http::request::Parts;
use axum::http::{HeaderMap, header};
use bytes::Bytes;
use iglu_api::Field;
use serde::de::DeserializeOwned;

use crate::app::{ApiError, Problem};

/// A JSON request body.
pub struct Body<T>(pub T);

impl<T, S> FromRequest<S> for Body<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        // Also why a cross-site form can't post here: it can't send this type.
        if !has_content_type(request.headers(), is_json) {
            return Err(ApiError::UnsupportedMediaType(
                "send the body as JSON, with Content-Type: application/json".into(),
            ));
        }
        let bytes = read(request, state).await?;
        parse_json(&bytes).map(Self).map_err(ApiError::BadRequest)
    }
}

/// A form a page of iglud's own posts.
pub struct Form<T>(pub T);

impl<T, S> FromRequest<S> for Form<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        if !has_content_type(request.headers(), |essence| {
            essence == "application/x-www-form-urlencoded"
        }) {
            return Err(ApiError::UnsupportedMediaType(
                "send the body as a form".into(),
            ));
        }
        let bytes = read(request, state).await?;
        parse_form(&bytes).map(Self).map_err(ApiError::BadRequest)
    }
}

/// A query string.
pub struct Query<T>(pub T);

impl<T, S> FromRequestParts<S> for Query<T>
where
    T: DeserializeOwned + Send,
    S: Send + Sync,
{
    type Rejection = ApiError;

    fn from_request_parts(
        parts: &mut Parts,
        _: &S,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send {
        let query = parts.uri.query().unwrap_or_default();
        std::future::ready(
            parse_form(query.as_bytes())
                .map(Self)
                .map_err(ApiError::BadRequest),
        )
    }
}

/// The request path's parameters.
pub struct Path<T>(pub T);

impl<T, S> FromRequestParts<S> for Path<T>
where
    T: DeserializeOwned + Send,
    S: Send + Sync,
{
    type Rejection = ApiError;

    #[expect(
        clippy::disallowed_types,
        reason = "this is the wrapper the lint points to"
    )]
    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let error = match axum::extract::Path::<T>::from_request_parts(parts, state).await {
            Ok(axum::extract::Path(value)) => return Ok(Self(value)),
            Err(PathRejection::FailedToDeserializePathParams(error)) => error.into_kind(),
            Err(rejection) => {
                tracing::error!(%rejection, "a route without parameters extracts them");
                return Err(ApiError::Internal);
            }
        };
        let keys: Vec<String> = RawPathParams::from_request_parts(parts, state)
            .await
            .map(|raw| raw.iter().map(|(key, _)| key.to_owned()).collect())
            .unwrap_or_default();
        match path_problem(error, &keys) {
            Ok(problem) => Err(ApiError::BadRequest(problem)),
            Err(error) => {
                tracing::error!(%error, "a route's parameters don't fit its handler");
                Err(ApiError::Internal)
            }
        }
    }
}

/// A WebSocket handshake.
#[expect(
    clippy::disallowed_types,
    reason = "this is the wrapper the lint points to"
)]
pub struct Upgrade(pub axum::extract::ws::WebSocketUpgrade);

impl<S> FromRequestParts<S> for Upgrade
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    #[expect(
        clippy::disallowed_types,
        reason = "this is the wrapper the lint points to"
    )]
    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        axum::extract::ws::WebSocketUpgrade::from_request_parts(parts, state)
            .await
            .map(Self)
            .map_err(|rejection| {
                ApiError::BadRequest(
                    format!("connect with a WebSocket: {}", rejection.body_text()).into(),
                )
            })
    }
}

async fn read<S: Send + Sync>(request: Request, state: &S) -> Result<Bytes, ApiError> {
    Bytes::from_request(request, state)
        .await
        .map_err(|rejection| {
            if rejection.status() == axum::http::StatusCode::PAYLOAD_TOO_LARGE {
                ApiError::TooLarge
            } else {
                ApiError::BadRequest("the request body couldn't be read".into())
            }
        })
}

fn has_content_type(headers: &HeaderMap, accepts: impl Fn(&str) -> bool) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|essence| accepts(&essence.trim().to_ascii_lowercase()))
}

fn is_json(essence: &str) -> bool {
    essence == "application/json"
        || essence
            .strip_prefix("application/")
            .is_some_and(|subtype| subtype.ends_with("+json"))
}

/// Parses a JSON body, or says what's wrong with it and where.
pub fn parse_json<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, Problem> {
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let value = serde_path_to_error::deserialize(&mut deserializer).map_err(|error| {
        let field = field_of(error.path());
        let error = error.into_inner();
        match error.classify() {
            serde_json::error::Category::Data => Problem {
                message: without_position(&error),
                field,
            },
            serde_json::error::Category::Syntax
            | serde_json::error::Category::Eof
            | serde_json::error::Category::Io => not_json(&error),
        }
    })?;
    deserializer.end().map_err(|error| not_json(&error))?;
    Ok(value)
}

fn not_json(error: &serde_json::Error) -> Problem {
    format!("the body isn't JSON: {}", without_position(error)).into()
}

/// `serde_json`'s message without its position, which means nothing to the
/// person reading it.
fn without_position(error: &serde_json::Error) -> String {
    let text = error.to_string();
    let position = format!(" at line {} column {}", error.line(), error.column());
    match text.strip_suffix(&position) {
        Some(message) => message.to_owned(),
        None => text,
    }
}

/// Parses a query string or form, or says what's wrong with it and where.
pub fn parse_form<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, Problem> {
    let deserializer = serde_urlencoded::Deserializer::new(url::form_urlencoded::parse(bytes));
    serde_path_to_error::deserialize(deserializer).map_err(|error| Problem {
        field: field_of(error.path()),
        message: error.into_inner().to_string(),
    })
}

/// The input at `path`, unless the error is about the whole request.
fn field_of(path: &serde_path_to_error::Path) -> Option<Field> {
    path.iter()
        .next()
        .is_some()
        .then(|| Field::new(path.to_string()))
}

/// What's wrong with a path parameter, and which one when that's known. A
/// parameter's own parser doesn't know its name; when the route has only
/// one, that's the one. `Err` means the route and handler disagree, which is
/// iglud's bug, not the caller's.
pub fn path_problem(error: PathErrorKind, keys: &[String]) -> Result<Problem, String> {
    let only = || match keys {
        [key] => Some(Field::new(key.as_str())),
        _ => None,
    };
    let (field, message) = match error {
        PathErrorKind::ParseErrorAtKey {
            key,
            value,
            expected_type,
        } => (
            Some(Field::new(key)),
            format!("{value:?} isn't a {expected_type}"),
        ),
        PathErrorKind::ParseErrorAtIndex {
            index,
            value,
            expected_type,
        } => (
            keys.get(index).map(|key| Field::new(key.as_str())),
            format!("{value:?} isn't a {expected_type}"),
        ),
        PathErrorKind::ParseError {
            value,
            expected_type,
        } => (only(), format!("{value:?} isn't a {expected_type}")),
        PathErrorKind::InvalidUtf8InPathParam { key } => {
            (Some(Field::new(key)), "isn't valid UTF-8".to_owned())
        }
        PathErrorKind::DeserializeError { key, message, .. } => (Some(Field::new(key)), message),
        PathErrorKind::Message(message) => (only(), message),
        PathErrorKind::WrongNumberOfParameters { got, expected } => {
            return Err(format!(
                "the route has {got} parameters, the handler wants {expected}"
            ));
        }
        PathErrorKind::UnsupportedType { name } => {
            return Err(format!("{name} can't be a path parameter"));
        }
        // A kind newer than this code; still a caller's mistake.
        other => (only(), other.to_string()),
    };
    Ok(Problem { message, field })
}

#[cfg(test)]
mod tests {
    use iglu_api::{ChangeProject, CreateEnvironment, PublishPort, PutSecret};
    use serde::Deserialize;

    use super::*;

    fn json_problem<T: DeserializeOwned>(json: &str) -> Problem {
        match parse_json::<T>(json.as_bytes()) {
            Ok(_) => panic!("{json} parsed"),
            Err(problem) => problem,
        }
    }

    #[test]
    fn a_field_that_doesnt_parse_is_named_without_serdes_position() {
        let problem =
            json_problem::<CreateEnvironment>(r#"{"name":"default","source":"github:a/b"}"#);
        assert_eq!(
            problem,
            Problem::at(
                "source",
                "invalid environment source: expected <flake>#<nixosConfigurations attribute>"
            )
        );
    }

    #[test]
    fn nested_fields_are_paths() {
        let project = r#"{"expected_revision":1,"name":"app","repo":null,"environment":"default","opening":[],"agent":null,"ports":[3000,0],"idle":{"kind":"default"}}"#;
        assert_eq!(
            json_problem::<ChangeProject>(project).field,
            Some(Field::new("ports[1]"))
        );
        let secret = r#"{"target":{"kind":"nope"},"value":"x"}"#;
        assert_eq!(
            json_problem::<PutSecret>(secret).field,
            Some(Field::new("target.kind"))
        );
    }

    #[test]
    fn missing_fields_are_about_the_whole_body() {
        let problem = json_problem::<CreateEnvironment>(r#"{"name":"default"}"#);
        assert_eq!(problem, Problem::from("missing field `source`"));
    }

    #[test]
    fn unknown_fields_are_named() {
        let problem = json_problem::<PublishPort>(r#"{"port":3000,"host":"x"}"#);
        assert_eq!(problem.field, Some(Field::new("host")));
    }

    #[test]
    fn a_body_that_isnt_json_says_so_without_naming_a_field() {
        for body in [r#"{"port":"#, "", r#"{"port":1} trailing"#, "not json"] {
            let problem = json_problem::<PublishPort>(body);
            assert!(
                problem.message.starts_with("the body isn't JSON: "),
                "{problem:?}"
            );
            assert!(!problem.message.contains(" at line "), "{problem:?}");
            assert_eq!(problem.field, None, "{body}");
        }
    }

    #[test]
    fn a_body_that_parses_is_its_value() {
        let port: PublishPort = parse_json(br#"{"port":3000}"#).expect("parses");
        assert_eq!(port.port.get(), 3000);
    }

    #[derive(Debug, Deserialize)]
    struct Size {
        #[expect(dead_code, reason = "only parsing is tested")]
        cols: u16,
    }

    #[test]
    fn query_strings_name_their_fields() {
        let problem = match parse_form::<Size>(b"cols=wide") {
            Ok(size) => panic!("parsed {size:?}"),
            Err(problem) => problem,
        };
        assert_eq!(problem.field, Some(Field::new("cols")));
        assert!(parse_form::<Size>(b"cols=80").is_ok());
    }

    #[test]
    fn a_parameters_own_message_is_put_on_the_routes_only_parameter() {
        let error = PathErrorKind::Message("invalid secret name: expected 1-64 of a-z".into());
        assert_eq!(
            path_problem(error, &["name".to_owned()]).expect("a caller's mistake"),
            Problem::at("name", "invalid secret name: expected 1-64 of a-z")
        );
        let error = PathErrorKind::Message("invalid session name".into());
        let keys = ["id".to_owned(), "session".to_owned()];
        assert_eq!(
            path_problem(error, &keys)
                .expect("a caller's mistake")
                .field,
            None
        );
    }

    #[test]
    fn parameters_that_name_themselves_are_named() {
        let error = PathErrorKind::DeserializeError {
            key: "id".into(),
            value: "nope".into(),
            message: "UUID parsing failed".into(),
        };
        assert_eq!(
            path_problem(error, &[]).expect("a caller's mistake"),
            Problem::at("id", "UUID parsing failed")
        );
        let error = PathErrorKind::ParseErrorAtIndex {
            index: 1,
            value: "x".into(),
            expected_type: "u16",
        };
        let keys = ["id".to_owned(), "port".to_owned()];
        assert_eq!(
            path_problem(error, &keys).expect("a caller's mistake"),
            Problem::at("port", r#""x" isn't a u16"#)
        );
    }

    #[test]
    fn a_route_that_doesnt_fit_its_handler_is_iglud_s_bug() {
        let error = PathErrorKind::WrongNumberOfParameters {
            got: 2,
            expected: 1,
        };
        assert!(path_problem(error, &[]).is_err());
    }

    #[test]
    fn json_content_types_include_suffixed_ones() {
        assert!(is_json("application/json"));
        assert!(is_json("application/problem+json"));
        assert!(!is_json("text/plain"));
        assert!(!is_json("application/x-www-form-urlencoded"));
    }
}
