//! Serves the demo apps over HTTP with Server-Sent Events, for trying the
//! browser. The protocol is described in [`hyperb_core::http`].
//!
//! ```text
//! cargo run -p hyperb-demoserver [addr]     # default 127.0.0.1:9001
//! ```
//!
//! Paths: `/restaurant` (also `/`), `/therapist`, `/textstats`, `/pomodoro`
//! (a tick-driven timer), `/qr`, `/chart` (images), `/billsplit`, `/finance`
//! (an RPN financial calculator) and `/meet` (a Doodle-like scheduler whose
//! meetings are shared between sessions and owned by keys), `/grammar`
//! (Harper), `/dbclient` (a SQL client over fake tables), `/minecraft` (a
//! fake server's admin console), `/saas` (a fake product's admin panel, kept
//! per key) and `/smarthome` (a simulated house). Each hello
//! starts a session with its own app instance, which ends when its event
//! stream closes (or if no stream is opened within [`STREAM_TIMEOUT`]).
//!
//! A session is anonymous and short-lived, but a visitor's profile is
//! remembered between sessions by their public key: the key arrives with the
//! hello, is injected into its signals as `__key`, and the apps load the
//! remembered profile from [`store`].
//!
//! The same apps are also served over iroh (see [`iroh`]); the key is then
//! the one of the iroh connection.

mod catalog;
mod common;
mod grammar;
mod iroh;
mod meet;
mod pomodoro;
mod restaurant;
mod saas;
mod smarthome;
mod store;
mod therapist;

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderName, Method, StatusCode, Uri, header};
use axum::response::Html;
use axum::response::sse::{self, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use demoapps::{billsplit, chart, dbclient, finance, img, minecraft, qr, textstats};
use futures_util::stream::{self, Stream, StreamExt};
use hyperb_core::http::{KEY, SESSION, SIGNATURE, line_to_event};
use hyperb_core::identity::{Identity, PublicKey};
use hypereffect::{App, Server, serde_json};
use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;
use tower_http::cors::{Any, CorsLayer};

const APPS: &[&str] = &[
    "restaurant",
    "therapist",
    "textstats",
    "pomodoro",
    "qr",
    "chart",
    "billsplit",
    "finance",
    "meet",
    "grammar",
    "dbclient",
    "minecraft",
    "saas",
    "smarthome",
];
const STREAM_TIMEOUT: Duration = Duration::from_secs(30);
/// The signal that carries the session key to the apps.
pub const KEY_SIGNAL: &str = "__key";

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
    let mut catalog = catalog::Catalog::default();
    let wasm_dir = std::env::var_os("HYPERB_WASM_DIR").unwrap_or_else(|| "wasm-apps".into());
    catalog.load_wasm(wasm_dir.as_ref());
    if catalog.wasm.is_empty() {
        eprintln!(
            "no wasm apps in {} (`just wasm-apps` builds them)",
            wasm_dir.to_string_lossy()
        );
    }
    match hyperb_core::identity::default_path()
        .map(|p| p.with_file_name("demoserver-iroh"))
        .ok_or_else(|| "no config directory".to_string())
        .and_then(|p| Identity::load_or_create(&p).map_err(|e| e.to_string()))
    {
        Ok(identity) => {
            catalog.iroh_id = Some(identity.public().to_string());
            tokio::spawn(iroh::serve(identity));
        }
        Err(e) => eprintln!("no iroh sessions: {e}"),
    }
    let listener = tokio::net::TcpListener::bind(&addr).await.expect("bind");
    print!("{}", catalog.listing(&addr));
    let _ = catalog::CATALOG.set(catalog);
    // Trim forgotten profiles now and then; getting a profile also refreshes
    // it lazily, this is only for visitors who never come back.
    tokio::spawn(async {
        let mut timer = tokio::time::interval(store::IDLE_TTL);
        loop {
            timer.tick().await;
            restaurant::GUESTS.prune();
            therapist::PATIENTS.prune();
            meet::prune();
            saas::WORKSPACES.prune();
        }
    });
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
        start(app, body, key, rx).await;
        sessions.lock().unwrap().remove(&sid);
        println!("session {app} for {key} ended");
    });
    ([(SESSION, id)], StatusCode::OK).into_response()
}

/// Opens a session's event stream.
async fn stream(State(sessions): State<Sessions>, uri: Uri, headers: HeaderMap) -> Response {
    if let Some(name) = uri.path().trim_matches('/').strip_suffix(".wasm") {
        return match catalog::CATALOG.get().and_then(|c| c.wasm.get(name)) {
            Some(wasm) => (
                [(header::CONTENT_TYPE, "application/wasm")],
                wasm.bytes.clone(),
            )
                .into_response(),
            None => (StatusCode::NOT_FOUND, "no wasm app here").into_response(),
        };
    }
    // Anything that is not a renderer asking for the event stream is a person
    // with a web browser.
    let wants_events = header_str(&headers, header::ACCEPT.as_str())
        .is_some_and(|a| a.contains("text/event-stream"));
    if !wants_events && app_at(&uri).is_some() {
        let host = header_str(&headers, header::HOST.as_str()).unwrap_or("localhost:9001");
        let page = catalog::CATALOG
            .get()
            .map(|c| c.page(host))
            .unwrap_or_default();
        return Html(page).into_response();
    }
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

/// Runs an instance of `app` until its stream closes.
async fn start(app: &str, hello: String, key: PublicKey, cmds: mpsc::UnboundedReceiver<Cmd>) {
    match app {
        "restaurant" => run::<restaurant::Restaurant>(hello, key, cmds).await,
        "therapist" => run::<therapist::Therapist>(hello, key, cmds).await,
        "pomodoro" => run::<pomodoro::Pomodoro>(hello, key, cmds).await,
        "qr" => run::<qr::Qr>(hello, key, cmds).await,
        "chart" => run::<chart::Chart>(hello, key, cmds).await,
        "billsplit" => run::<billsplit::BillSplit>(hello, key, cmds).await,
        "finance" => run::<finance::Finance>(hello, key, cmds).await,
        "meet" => run::<meet::Meet>(hello, key, cmds).await,
        "grammar" => run::<grammar::Grammar>(hello, key, cmds).await,
        "dbclient" => run::<dbclient::DbClient>(hello, key, cmds).await,
        "minecraft" => run::<minecraft::Minecraft>(hello, key, cmds).await,
        "saas" => run::<saas::Saas>(hello, key, cmds).await,
        "smarthome" => run::<smarthome::SmartHome>(hello, key, cmds).await,
        _ => run::<textstats::TextStats>(hello, key, cmds).await,
    }
}

/// Runs one app instance until its stream closes.
async fn run<A: App + Default>(
    hello: String,
    key: PublicKey,
    mut cmds: mpsc::UnboundedReceiver<Cmd>,
) {
    let mut server = Server::<A>::new();
    receive(&mut server, with_key(&hello, key));
    // Output from before the stream opened.
    let mut backlog = String::new();
    let mut out: Option<mpsc::UnboundedSender<String>> = None;
    let mut ticker: Option<(u32, tokio::time::Interval)> = None;
    let deadline = tokio::time::sleep(STREAM_TIMEOUT);
    tokio::pin!(deadline);
    loop {
        backlog.push_str(&server.output().take());
        if let Some(tx) = &out
            && !backlog.is_empty()
            && tx.send(std::mem::take(&mut backlog)).is_err()
        {
            break;
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
                Some(Cmd::Input(lines)) => receive(&mut server, with_key(&lines, key)),
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

/// Injects the session key into a hello's params or an action's signals as
/// [`KEY_SIGNAL`], so the app knows whose session it is and can load the
/// remembered profile. The renderer never sends the key (it keeps
/// `_`-prefixed signals local), and any `KEY_SIGNAL` param it sends is
/// replaced, so this is the only way the apps see it. Lines that are not
/// hello or action lines pass through unchanged.
fn with_key(lines: &str, key: PublicKey) -> String {
    let mut out = String::with_capacity(lines.len());
    for line in lines.lines() {
        out.push_str(&line_with_key(line, key));
        out.push('\n');
    }
    out
}

fn line_with_key(line: &str, key: PublicKey) -> String {
    use serde_json::Value;
    let Some((tag, body)) = line.split_once(' ') else {
        return line.to_owned();
    };
    if tag != "hello" && tag != "action" {
        return line.to_owned();
    }
    let mut body = serde_json::from_str::<Value>(body).unwrap_or(Value::Null);
    if let Value::Object(ref mut body) = body {
        let field = if tag == "hello" { "params" } else { "signals" };
        let map = body
            .entry(field)
            .or_insert_with(|| Value::Object(Default::default()));
        if let Value::Object(map) = map {
            map.insert(KEY_SIGNAL.into(), key.to_string().into());
        }
    }
    format!("{tag} {body}")
}

fn receive<A: App + Default>(server: &mut Server<A>, mut lines: String) {
    if !lines.ends_with('\n') {
        lines.push('\n');
    }
    server.receive(lines.as_bytes());
}

fn app_at(uri: &Uri) -> Option<&'static str> {
    app_at_path(uri.path())
}

fn app_at_path(path: &str) -> Option<&'static str> {
    match path.trim_matches('/') {
        "" => Some("restaurant"),
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

/// Helpers for testing an app the way the transport drives it: with the
/// session key injected into every line the client sends.
#[cfg(test)]
mod testutil {
    use super::*;
    use hypereffect::server::Loopback;

    pub fn connect<A: App + Default>(key: PublicKey) -> Loopback<A> {
        let mut l = Loopback {
            client: hypereffect::client::Client::new(),
            server: Server::new(),
        };
        let hello = l.client.hello();
        l.server.receive(with_key(&hello, key).as_bytes());
        l.pump();
        l
    }

    /// Triggers a handler, if the client lets it, and delivers the response.
    pub fn trigger<A: App + Default>(
        l: &mut Loopback<A>,
        key: PublicKey,
        id: &str,
        event: &str,
    ) -> bool {
        let Some(line) = l.client.trigger(id, event) else {
            return false;
        };
        l.server.receive(with_key(&line, key).as_bytes());
        l.pump();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_goes_into_hello_params_and_action_signals() {
        let key = hyperb_core::identity::Identity::generate().public();
        assert_eq!(
            with_key(r#"hello {"params":{"page":"home"}}"#, key),
            format!("hello {{\"params\":{{\"page\":\"home\",\"__key\":\"{key}\"}}}}\n")
        );
        assert_eq!(
            with_key(r#"action {"name":"x","signals":{"page":"home"}}"#, key),
            format!(
                "action {{\"name\":\"x\",\"signals\":{{\"page\":\"home\",\"__key\":\"{key}\"}}}}\n"
            )
        );
        // A body without signals gets one.
        assert_eq!(
            with_key(r#"action {"name":"x"}"#, key),
            format!("action {{\"name\":\"x\",\"signals\":{{\"__key\":\"{key}\"}}}}\n")
        );
        // Other tags, and malformed bodies: unchanged.
        assert_eq!(with_key("signal {\"n\":1}", key), "signal {\"n\":1}\n");
        assert_eq!(with_key("hello [1,2]", key), "hello [1,2]\n");
    }
}
