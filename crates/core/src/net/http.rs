//! The renderer side of [`crate::http`], on reqwest (fetch on wasm32).

use crate::http::{KEY, SESSION, SIGNATURE, event_to_line};
use crate::identity::Identity;
use crate::net::Event;
use crate::sse;
use futures_util::{StreamExt, future, pin_mut};
use reqwest::header::{ACCEPT, CONTENT_TYPE};
use reqwest::{Client, RequestBuilder, Response};
use url::Url;

pub(super) async fn run(
    url: Url,
    identity: Identity,
    out: async_channel::Receiver<String>,
    events: &async_channel::Sender<Event>,
) -> Result<(), String> {
    let _ = events.send(Event::Open).await;
    let Ok(hello) = out.recv().await else {
        return Ok(());
    };
    #[cfg(target_os = "android")]
    let _ = rustls::crypto::ring::default_provider().install_default();
    let http = Client::new();
    let key = identity.public().to_string();
    let post = |line: String| {
        http.post(url.clone())
            .header(KEY, &key)
            .header(CONTENT_TYPE, "text/plain; charset=utf-8")
            .body(line)
    };

    let resp = check(post(hello).send().await).await?;
    let session = resp
        .headers()
        .get(SESSION)
        .and_then(|v| v.to_str().ok())
        .ok_or("the server did not start a session")?
        .to_string();
    let signature = identity.sign(session.as_bytes());
    let auth = |req: RequestBuilder| req.header(SESSION, &session).header(SIGNATURE, &signature);

    let stream = auth(http.get(url.clone()).header(KEY, &key))
        .header(ACCEPT, "text/event-stream")
        .send()
        .await;
    let stream = check(stream).await?;
    let is_sse = stream
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("text/event-stream"));
    if !is_sse {
        return Err("the server did not answer with an event stream".into());
    }

    let read = async {
        let mut body = stream.bytes_stream();
        let mut parser = sse::Parser::new();
        while let Some(chunk) = body.next().await {
            // reqwest calls a dropped stream "error decoding response body".
            let chunk = chunk.map_err(|_| "connection lost".to_string())?;
            for ev in parser.push(&chunk) {
                let _ = events
                    .send(Event::Data(event_to_line(&ev.event, &ev.data)))
                    .await;
            }
        }
        Ok(())
    };
    // One POST at a time, so the server sees lines in order.
    let write = async {
        while let Ok(line) = out.recv().await {
            check(auth(post(line)).send().await).await?;
        }
        Ok(())
    };
    pin_mut!(read, write);
    future::select(read, write).await.factor_first().0
}

/// The response if it succeeded, or the error with the server's explanation.
async fn check(resp: reqwest::Result<Response>) -> Result<Response, String> {
    let resp = resp.map_err(|e| e.to_string())?;
    let status = resp.status();
    if status.is_success() {
        return Ok(resp);
    }
    let text = resp.text().await.unwrap_or_default();
    let text = text.trim();
    Err(if text.is_empty() {
        status.to_string()
    } else {
        format!("{status}: {text}")
    })
}
