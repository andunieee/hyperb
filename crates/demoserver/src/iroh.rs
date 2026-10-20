//! The apps over iroh (`SPEC.md`, section 3). The ALPN names the app and the
//! connection's endpoint key is the visitor's key, so there are no sessions
//! to open or to authenticate: one bidirectional stream carries the lines of
//! one app instance, which ends with the stream.

use crate::{app_at_path, start, Cmd, APPS};
use hyperb_core::address::{iroh_alpn_path, IROH_ALPN};
use hyperb_core::identity::{Identity, PublicKey};
use iroh::endpoint::{presets, Connection, Incoming};
use iroh::{Endpoint, SecretKey};
use tokio::sync::mpsc;

pub async fn serve(identity: Identity) {
    // Each app is its own ALPN; the root path is the same as /restaurant.
    let alpns = std::iter::once(IROH_ALPN.to_string())
        .chain(APPS.iter().map(|a| format!("{IROH_ALPN}/{a}")))
        .map(String::into_bytes)
        .collect();
    let endpoint = match Endpoint::builder(presets::N0)
        .secret_key(SecretKey::from_bytes(&identity.secret()))
        .alpns(alpns)
        .bind()
        .await
    {
        Ok(e) => e,
        Err(e) => return eprintln!("no iroh sessions: {e}"),
    };
    endpoint.online().await;
    while let Some(incoming) = endpoint.accept().await {
        tokio::spawn(async move {
            if let Err(e) = session(incoming).await {
                eprintln!("iroh session: {e}");
            }
        });
    }
}

async fn session(incoming: Incoming) -> Result<(), String> {
    let conn: Connection = incoming
        .accept()
        .map_err(|e| e.to_string())?
        .await
        .map_err(|e| e.to_string())?;
    let app = iroh_alpn_path(conn.alpn())
        .as_deref()
        .and_then(|p| app_at_path(p.split('?').next().unwrap_or_default()))
        .ok_or("no app here")?;
    let key: PublicKey = conn.remote_id().to_string().parse().map_err(|_| "bad key")?;
    let (mut send, mut recv) = conn.accept_bi().await.map_err(|e| e.to_string())?;
    println!("iroh session {app} for {key}");

    let (cmds, rx) = mpsc::unbounded_channel();
    let (out, mut out_rx) = mpsc::unbounded_channel();
    let _ = cmds.send(Cmd::Stream(out));
    // The first line is the hello, which starts the app.
    let mut lines = Lines::default();
    let hello = loop {
        if let Some(line) = lines.next() {
            break line;
        }
        if !lines.fill(&mut recv).await? {
            return Ok(());
        }
    };
    let app_task = tokio::spawn(start(app, hello, key, rx));

    let read = async {
        loop {
            while let Some(line) = lines.next() {
                if cmds.send(Cmd::Input(line)).is_err() {
                    return Ok(());
                }
            }
            if !lines.fill(&mut recv).await? {
                return Ok::<_, String>(());
            }
        }
    };
    let write = async {
        while let Some(chunk) = out_rx.recv().await {
            send.write_all(chunk.as_bytes())
                .await
                .map_err(|e| e.to_string())?;
        }
        // The app ended: finish the stream so the renderer sees it close.
        let _ = send.finish();
        let _ = conn.closed().await;
        Ok::<_, String>(())
    };
    let result = tokio::select! { r = read => r, r = write => r };
    drop(cmds);
    let _ = app_task.await;
    println!("iroh session {app} for {key} ended");
    result
}

/// Lines of a byte stream.
#[derive(Default)]
struct Lines(Vec<u8>);

impl Lines {
    fn next(&mut self) -> Option<String> {
        let end = self.0.iter().position(|&b| b == b'\n')?;
        let line: Vec<u8> = self.0.drain(..=end).collect();
        Some(String::from_utf8_lossy(&line).into_owned())
    }

    /// Reads more; false at the end of the stream.
    async fn fill(&mut self, recv: &mut iroh::endpoint::RecvStream) -> Result<bool, String> {
        let mut buf = [0u8; 16 * 1024];
        match recv.read(&mut buf).await {
            Ok(Some(n)) => {
                self.0.extend_from_slice(&buf[..n]);
                Ok(true)
            }
            Ok(None) => Ok(false),
            Err(e) => Err(e.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyperb_core::address;
    use hyperb_core::net::{Connection, Event};
    use std::time::Duration;

    /// Dials through iroh's default discovery, so it needs internet access:
    /// `cargo test -p hyperb-demoserver -- --ignored`.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn renderer_and_app_over_iroh() {
        let server = Identity::generate();
        tokio::spawn(serve(server.clone()));
        // Let it publish its address.
        tokio::time::sleep(Duration::from_secs(10)).await;
        let url = address::normalize(&format!("hyperb://{}/textstats", server.public())).unwrap();
        let conn = Connection::open(&url, &Identity::generate());
        let events = conn.events();
        let next = || async {
            tokio::time::timeout(Duration::from_secs(60), events.recv())
                .await
                .expect("timeout")
                .unwrap()
        };
        assert_eq!(next().await, Event::Open);
        conn.send("hello {}\n".into());
        match next().await {
            Event::Data(line) => assert!(line.ends_with('\n') && line.contains(' '), "{line}"),
            e => panic!("{e:?}"),
        }
    }
}
