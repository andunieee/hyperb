//! The renderer side of an iroh session (`SPEC.md`, section 3).

use crate::address;
use crate::identity::Identity;
use crate::net::Event;
use futures_util::{future, pin_mut};
use iroh::endpoint::presets;
use iroh::{Endpoint, EndpointId, SecretKey};
use url::Url;

pub(super) async fn run(
    url: Url,
    identity: Identity,
    out: async_channel::Receiver<String>,
    events: &async_channel::Sender<Event>,
) -> Result<(), String> {
    let id: EndpointId = url
        .host_str()
        .unwrap_or_default()
        .parse()
        .map_err(|e| format!("bad iroh endpoint id: {e}"))?;
    let alpn = address::iroh_alpn(&url)?;
    let _ = events.send(Event::Open).await;
    let Ok(hello) = out.recv().await else {
        return Ok(());
    };
    // The identity is the endpoint's key: the server learns it from the
    // connection, with no headers or signatures.
    let endpoint = Endpoint::builder(presets::N0)
        .secret_key(SecretKey::from_bytes(&identity.secret()))
        .bind()
        .await
        .map_err(|e| e.to_string())?;
    let conn = endpoint
        .connect(id, &alpn)
        .await
        .map_err(|e| format!("could not connect: {e}"))?;
    let (mut send, mut recv) = conn.open_bi().await.map_err(|e| e.to_string())?;
    send.write_all(hello.as_bytes())
        .await
        .map_err(|e| e.to_string())?;

    let read = async {
        let (mut buf, mut pending) = (vec![0u8; 16 * 1024], Vec::new());
        loop {
            let n = match recv.read(&mut buf).await {
                Ok(Some(n)) => n,
                Ok(None) => return Ok(()),
                Err(_) => return Err("connection lost".to_string()),
            };
            pending.extend_from_slice(&buf[..n]);
            while let Some(end) = pending.iter().position(|&b| b == b'\n') {
                let line: Vec<u8> = pending.drain(..=end).collect();
                let line = String::from_utf8_lossy(&line).into_owned();
                let _ = events.send(Event::Data(line)).await;
            }
        }
    };
    // Stream writes are ordered, so lines go out as they are queued.
    let write = async {
        while let Ok(line) = out.recv().await {
            send.write_all(line.as_bytes())
                .await
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    };
    pin_mut!(read, write);
    let result = future::select(read, write).await.factor_first().0;
    conn.close(0u32.into(), b"bye");
    endpoint.close().await;
    result
}
