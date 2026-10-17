//! Serves the hypereffect demo apps over HTTP with Server-Sent Events, for
//! trying the browser. The protocol is described in [`hyperb_core::http`].
//!
//! ```text
//! cargo run -p hyperb-devserver [addr]     # default 127.0.0.1:9001
//! ```
//!
//! Paths: `/todos` (also `/`), `/psu`, `/import`. Each hello starts a session
//! with its own app instance, which ends when its event stream closes (or
//! if no stream is opened within [`STREAM_TIMEOUT`]).

use axum::extract::State;
use axum::http::{header, HeaderMap, HeaderName, Method, StatusCode, Uri};
use axum::response::sse::{self, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use futures_util::stream::{self, Stream, StreamExt};
use hyperb_core::http::{line_to_event, KEY, SESSION, SIGNATURE};
use hyperb_core::identity::PublicKey;
use hypereffect::{App, Server};
use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;
use tower_http::cors::{Any, CorsLayer};

const APPS: &[&str] = &["todos", "psu", "import"];
const STREAM_TIMEOUT: Duration = Duration::from_secs(30);

type Sessions = Arc<Mutex<HashMap<String, Session>>>;

struct Session {
    app: &'static str,
    key: PublicKey,
    cmds: mpsc::UnboundedSender<Cmd>,
    streaming: bool,
}

enum Cmd {
    /// Lines from the renderer.
    Input(String),
    /// The event stream opened; send everything there.
    Stream(mpsc::UnboundedSender<String>),
}

#[tokio::main]
async fn main() {
    let addr = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:9001".into());
    let headers = [KEY, SESSION, SIGNATURE].map(HeaderName::from_static);
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET, Method::POST])
        .allow_headers([
            headers[0].clone(),
            headers[1].clone(),
            headers[2].clone(),
            header::CONTENT_TYPE,
        ])
        .expose_headers([headers[1].clone()]);
    let app = Router::new()
        .route("/", get(stream).post(post))
        .route("/{app}", get(stream).post(post))
        .layer(cors)
        .with_state(Sessions::default());
    let listener = tokio::net::TcpListener::bind(&addr).await.expect("bind");
    for app in APPS {
        println!("http://{addr}/{app}");
    }
    axum::serve(listener, app).await.expect("serve");
}

/// A hello (no session yet) or a line for a session.
async fn post(
    State(sessions): State<Sessions>,
    uri: Uri,
    headers: HeaderMap,
    body: String,
) -> Response {
    let Some(app) = app_at(&uri) else {
        return (StatusCode::NOT_FOUND, "no app here").into_response();
    };
    if headers.contains_key(SESSION) {
        let mut map = sessions.lock().unwrap();
        return match authorize(&mut map, app, &headers) {
            Ok(s) if s.cmds.send(Cmd::Input(body)).is_ok() => {
                StatusCode::NO_CONTENT.into_response()
            }
            Ok(_) => (StatusCode::GONE, "session ended").into_response(),
            Err(r) => r.into_response(),
        };
    }
    let Some(key) = header_str(&headers, KEY).and_then(|k| k.parse::<PublicKey>().ok()) else {
        return (StatusCode::BAD_REQUEST, format!("missing or invalid {KEY}")).into_response();
    };
    let id = session_id();
    let (cmds, rx) = mpsc::unbounded_channel();
    sessions.lock().unwrap().insert(
        id.clone(),
        Session {
            app,
            key,
            cmds,
            streaming: false,
        },
    );
    println!("session {app} for {key}");
    let (sessions, sid) = (sessions.clone(), id.clone());
    tokio::spawn(async move {
        match app {
            "todos" => run::<todos::Todos>(body, rx).await,
            "psu" => run::<psu::Psu>(body, rx).await,
            _ => run::<import::Import>(body, rx).await,
        }
        sessions.lock().unwrap().remove(&sid);
        println!("session {app} for {key} ended");
    });
    ([(SESSION, id)], StatusCode::OK).into_response()
}

/// Opens a session's event stream.
async fn stream(State(sessions): State<Sessions>, uri: Uri, headers: HeaderMap) -> Response {
    let Some(app) = app_at(&uri) else {
        return (StatusCode::NOT_FOUND, "no app here").into_response();
    };
    let mut map = sessions.lock().unwrap();
    let s = match authorize(&mut map, app, &headers) {
        Ok(s) => s,
        Err(r) => return r.into_response(),
    };
    if s.streaming {
        return (StatusCode::CONFLICT, "the session already has a stream").into_response();
    }
    let (tx, rx) = mpsc::unbounded_channel();
    if s.cmds.send(Cmd::Stream(tx)).is_err() {
        return (StatusCode::GONE, "session ended").into_response();
    }
    s.streaming = true;
    Sse::new(events(rx))
        .keep_alive(KeepAlive::default())
        .into_response()
}

/// One SSE event per line, named by its tag.
fn events(
    rx: mpsc::UnboundedReceiver<String>,
) -> impl Stream<Item = Result<sse::Event, Infallible>> {
    stream::unfold(rx, |mut rx| async move { Some((rx.recv().await?, rx)) }).flat_map(|chunk| {
        let events: Vec<_> = chunk
            .lines()
            .filter_map(line_to_event)
            .map(|(tag, json)| Ok(sse::Event::default().event(tag).data(json)))
            .collect();
        stream::iter(events)
    })
}

/// The session the auth headers name, if they are valid for it.
fn authorize<'a>(
    map: &'a mut HashMap<String, Session>,
    app: &str,
    headers: &HeaderMap,
) -> Result<&'a mut Session, (StatusCode, &'static str)> {
    let deny = |why| Err((StatusCode::UNAUTHORIZED, why));
    let id = header_str(headers, SESSION).unwrap_or_default();
    let Some(s) = map.get_mut(id) else {
        return deny("unknown session");
    };
    let key = header_str(headers, KEY).and_then(|k| k.parse::<PublicKey>().ok());
    if s.app != app || key != Some(s.key) {
        return deny("the session belongs to another key or app");
    }
    if !s.key.verify(
        id.as_bytes(),
        header_str(headers, SIGNATURE).unwrap_or_default(),
    ) {
        return deny("bad signature");
    }
    Ok(s)
}

/// Runs one app instance until its stream closes.
async fn run<A: App + Default>(hello: String, mut cmds: mpsc::UnboundedReceiver<Cmd>) {
    let mut server = Server::<A>::new();
    receive(&mut server, hello);
    // Output from before the stream opened.
    let mut backlog = String::new();
    let mut out: Option<mpsc::UnboundedSender<String>> = None;
    let mut ticker: Option<(u32, tokio::time::Interval)> = None;
    let deadline = tokio::time::sleep(STREAM_TIMEOUT);
    tokio::pin!(deadline);
    loop {
        backlog.push_str(&server.output().take());
        if let Some(tx) = &out {
            if !backlog.is_empty() && tx.send(std::mem::take(&mut backlog)).is_err() {
                break;
            }
        }
        let want = server.tick_interval();
        if want != ticker.as_ref().map(|t| t.0) {
            ticker = want.map(|ms| {
                let d = Duration::from_millis(ms.max(1).into());
                (
                    ms,
                    tokio::time::interval_at(tokio::time::Instant::now() + d, d),
                )
            });
        }
        tokio::select! {
            cmd = cmds.recv() => match cmd {
                Some(Cmd::Input(lines)) => receive(&mut server, lines),
                Some(Cmd::Stream(tx)) => out = Some(tx),
                None => break,
            },
            _ = async { ticker.as_mut().unwrap().1.tick().await }, if ticker.is_some() => {
                server.tick(now_ms());
            }
            _ = &mut deadline, if out.is_none() => break,
            _ = async { out.as_ref().unwrap().closed().await }, if out.is_some() => break,
        }
    }
    server.close();
}

fn receive<A: App + Default>(server: &mut Server<A>, mut lines: String) {
    if !lines.ends_with('\n') {
        lines.push('\n');
    }
    server.receive(lines.as_bytes());
}

fn app_at(uri: &Uri) -> Option<&'static str> {
    match uri.path().trim_matches('/') {
        "" => Some("todos"),
        p => APPS.iter().copied().find(|a| *a == p),
    }
}

fn header_str<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name)?.to_str().ok()
}

fn session_id() -> String {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).expect("system random number generator");
    hex::encode(bytes)
}

fn now_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
        * 1000.0
}
