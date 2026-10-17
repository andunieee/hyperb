//! The HTTP transport: Server-Sent Events from the server, POSTs from the
//! renderer (spec section 11), with sessions bound to the renderer's
//! [`Identity`](crate::identity::Identity). Everything happens on the app's
//! URL:
//!
//! 1. `POST` with the `hello` line as the body and [`KEY`] set to the
//!    renderer's public key. The server starts a session and answers 200
//!    with its id in [`SESSION`].
//! 2. `GET` with `Accept: text/event-stream` and the auth headers: [`KEY`],
//!    [`SESSION`] and [`SIGNATURE`], the renderer's signature of the session
//!    id. Every message from the server arrives on this stream as one SSE
//!    event, named by the tag, whose data is the JSON. The session ends when
//!    the stream closes.
//! 3. Every other message from the renderer is a `POST` with the auth headers
//!    and the line as the body.
//!
//! Servers should answer requests with missing or wrong auth with 401.

/// The renderer's public key, as hex.
pub const KEY: &str = "hyperb-key";
/// The session id the server gave in answer to the hello.
pub const SESSION: &str = "hyperb-session";
/// The renderer's signature of the session id (its UTF-8 bytes), as hex.
pub const SIGNATURE: &str = "hyperb-signature";

/// Splits one line from a server into the SSE event name and data.
pub fn line_to_event(line: &str) -> Option<(&str, &str)> {
    let line = line.trim_end_matches(['\n', '\r']);
    line.split_once(' ').filter(|(tag, _)| !tag.is_empty())
}

/// The line an SSE event stands for, with its line feed.
pub fn event_to_line(event: &str, data: &str) -> String {
    format!("{event} {data}\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_and_events() {
        let line = "element {\"id\":\"a b\"}\n";
        let (event, data) = line_to_event(line).unwrap();
        assert_eq!((event, data), ("element", "{\"id\":\"a b\"}"));
        assert_eq!(event_to_line(event, data), line);
        assert_eq!(line_to_event("nospace"), None);
    }
}
