//! Transports for hypereffect sessions, chosen by the URL scheme. Only HTTP
//! ([`crate::http`]) exists so far.
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

use crate::identity::Identity;
use futures_util::future::{AbortHandle, Abortable};
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
    out: async_channel::Sender<String>,
    events: async_channel::Receiver<Event>,
    abort: AbortHandle,
}

impl Connection {
    /// Opens a session at `url`, authenticated as `identity`.
    pub fn open(url: &Url, identity: &Identity) -> Self {
        let (out, out_rx) = async_channel::unbounded();
        let (ev_tx, events) = async_channel::unbounded();
        let (abort, reg) = AbortHandle::new_pair();
        let (url, identity) = (url.clone(), identity.clone());
        spawn(async move {
            let run = async {
                match url.scheme() {
                    "http" | "https" => http::run(url, identity, out_rx, &ev_tx).await,
                    s => Err(format!("no transport for '{s}'")),
                }
            };
            if let Ok(result) = Abortable::new(run, reg).await {
                let _ = ev_tx.send(Event::Closed(result.err())).await;
            }
        });
        Connection { out, events, abort }
    }

    /// Queues a line to send. Ignored once the connection has closed.
    pub fn send(&self, line: String) {
        let _ = self.out.try_send(line);
    }

    pub fn events(&self) -> async_channel::Receiver<Event> {
        self.events.clone()
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.abort.abort();
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn spawn(fut: impl Future<Output = ()> + Send + 'static) {
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
fn spawn(fut: impl Future<Output = ()> + 'static) {
    wasm_bindgen_futures::spawn_local(fut);
}
