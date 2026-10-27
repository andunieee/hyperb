//! Transports for hypereffect sessions, chosen by the URL scheme that
//! [`crate::address::normalize`] produces: `http`/`https` ([`crate::http`]),
//! `iroh` and `wasm` (features `iroh` and `wasm`; see `SPEC.md`).
//!
//! Every transport behaves the same to the renderer: after [`Event::Open`]
//! the first line sent must be the `hello`, and lines from the server arrive
//! as [`Event::Data`]. Events arrive on an [`async_channel::Receiver`], which
//! any executor (such as the GLib main loop) can await. Dropping a
//! [`Connection`] closes it.
//!
//! On native hosts transports run on a shared background tokio runtime; on
//! wasm32 they run on the browser's event loop and use fetch.

mod http;
#[cfg(feature = "iroh")]
mod iroh;
#[cfg(all(feature = "wasm", not(target_arch = "wasm32")))]
mod wasm;
#[cfg(all(feature = "wasm", target_arch = "wasm32"))]
#[path = "wasm_js.rs"]
mod wasm;

use crate::address::{self, Kind};
use crate::identity::Identity;
use futures_util::future::{AbortHandle, Abortable};
use std::collections::BTreeMap;
use std::future::Future;
use url::Url;

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// The transport is open; the renderer must now send hello.
    Open,
    /// Lines from the server, always ending with a line feed.
    Data(String),
    /// The transport closed, with the reason if it was an error.
    Closed(Option<String>),
}

pub struct Connection {
    params: BTreeMap<String, String>,
    out: async_channel::Sender<String>,
    events: async_channel::Receiver<Event>,
    abort: AbortHandle,
}

impl Connection {
    /// Opens a session at `url`, authenticated as `identity`.
    pub fn open(url: &Url, identity: &Identity) -> Self {
        Self::start(url, Source::Identity(identity.clone()))
    }

    /// Opens a wasm app session at `url` that runs `module` instead of
    /// loading it, such as a module kept by the [`crate::library`].
    #[cfg(feature = "wasm")]
    pub fn open_module(url: &Url, module: Vec<u8>) -> Self {
        Self::start(url, Source::Module(module))
    }

    fn start(url: &Url, source: Source) -> Self {
        let (out, out_rx) = async_channel::unbounded();
        let (ev_tx, events) = async_channel::unbounded();
        let (abort, reg) = AbortHandle::new_pair();
        let params = address::params(url);
        let url = url.clone();
        spawn(async move {
            let run = async {
                match (address::kind(&url), source) {
                    #[cfg(feature = "wasm")]
                    (Kind::LocalWasm | Kind::RemoteWasm, source) => {
                        let module = match source {
                            Source::Module(m) => Some(m),
                            Source::Identity(_) => None,
                        };
                        wasm::run(url, module, out_rx, &ev_tx).await
                    }
                    (Kind::Http, Source::Identity(id)) => http::run(url, id, out_rx, &ev_tx).await,
                    #[cfg(feature = "iroh")]
                    (Kind::Iroh, Source::Identity(id)) => iroh::run(url, id, out_rx, &ev_tx).await,
                    #[allow(unreachable_patterns)]
                    (k, Source::Identity(_)) => {
                        Err(format!("{k:?} sessions are not available in this build"))
                    }
                    #[allow(unreachable_patterns)]
                    (_, _) => Err("not a wasm app".into()),
                }
            };
            if let Ok(result) = Abortable::new(run, reg).await {
                let _ = ev_tx.send(Event::Closed(result.err())).await;
            }
        });
        Connection {
            params,
            out,
            events,
            abort,
        }
    }

    /// The session parameters of the address, which the `hello` carries
    /// (`Client::hello_with`).
    pub fn params(&self) -> &BTreeMap<String, String> {
        &self.params
    }

    /// Queues a line to send. Ignored once the connection has closed.
    pub fn send(&self, line: String) {
        let _ = self.out.try_send(line);
    }

    pub fn events(&self) -> async_channel::Receiver<Event> {
        self.events.clone()
    }
}

/// What a session starts from: the renderer's identity, or for wasm apps
/// (which have none) possibly the module to run.
enum Source {
    Identity(Identity),
    #[cfg(feature = "wasm")]
    Module(Vec<u8>),
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.abort.abort();
    }
}

/// Largest wasm module accepted when downloading.
const MAX_MODULE: usize = 64 << 20;

/// Downloads the module of the remote wasm app at `url` (at most 64 MiB).
/// Any executor may await it.
pub async fn fetch_module(url: &Url) -> Result<Vec<u8>, String> {
    if address::kind(url) != Kind::RemoteWasm {
        return Err("not a remote wasm app".into());
    }
    // reqwest needs the runtime's reactor, so it runs there.
    let (tx, rx) = async_channel::bounded(1);
    let url = url.clone();
    spawn(async move {
        let _ = tx.send(download(&url).await).await;
    });
    rx.recv()
        .await
        .map_err(|_| "the download was cancelled".to_string())?
}

async fn download(url: &Url) -> Result<Vec<u8>, String> {
    #[cfg(target_os = "android")]
    let _ = rustls::crypto::ring::default_provider().install_default();
    let resp = reqwest::get(url.clone())
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| e.to_string())?;
    if resp.content_length().is_some_and(|n| n > MAX_MODULE as u64) {
        return Err("the wasm app is too large".into());
    }
    let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
    if bytes.len() > MAX_MODULE {
        return Err("the wasm app is too large".into());
    }
    Ok(bytes.to_vec())
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn spawn(fut: impl Future<Output = ()> + Send + 'static) {
    use std::sync::OnceLock;
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_name("hyperb-net")
            .enable_all()
            .build()
            .expect("tokio runtime")
    })
    .spawn(fut);
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn spawn(fut: impl Future<Output = ()> + 'static) {
    wasm_bindgen_futures::spawn_local(fut);
}
