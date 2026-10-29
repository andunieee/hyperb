//! What the demoserver serves and the `hyperb://` addresses to open it with,
//! printed at startup and listed on the page for ordinary web browsers.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub const REPO: &str = "https://github.com/hypereffect/hyperb";

pub static CATALOG: OnceLock<Catalog> = OnceLock::new();

#[derive(Default)]
pub struct Catalog {
    /// The endpoint id the apps are served on over iroh, if they are.
    pub iroh_id: Option<String>,
    /// Wasm modules, by app name.
    pub wasm: BTreeMap<String, Wasm>,
}

pub struct Wasm {
    pub path: PathBuf,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Http,
    Iroh,
    RemoteWasm,
}

impl Kind {
    const ALL: [Kind; 3] = [Kind::Http, Kind::Iroh, Kind::RemoteWasm];

    pub fn title(self) -> &'static str {
        match self {
            Kind::Http => "Over HTTP",
            Kind::Iroh => "Over iroh",
            Kind::RemoteWasm => "As wasm apps, downloaded from this server",
        }
    }
}

pub struct Entry {
    pub kind: Kind,
    pub url: String,
}

impl Catalog {
    /// Loads the `.wasm` files in `dir`; there are none if it does not exist.
    pub fn load_wasm(&mut self, dir: &Path) {
        let Ok(files) = std::fs::read_dir(dir) else {
            return;
        };
        for file in files.flatten() {
            let path = file.path();
            let name = path.file_stem().and_then(|n| n.to_str()).map(str::to_owned);
            let (Some(name), Some("wasm")) = (name, path.extension().and_then(|e| e.to_str()))
            else {
                continue;
            };
            if let Ok(bytes) = std::fs::read(&path) {
                let path = std::fs::canonicalize(&path).unwrap_or(path);
                self.wasm.insert(name, Wasm { path, bytes });
            }
        }
    }

    /// The addresses to open the apps with, for a client that reaches the
    /// HTTP server at `host` (with its port).
    pub fn entries(&self, host: &str) -> Vec<Entry> {
        let mut out = Vec::new();
        for app in crate::APPS {
            out.push(Entry {
                kind: Kind::Http,
                url: format!("hyperb://{host}/{app}"),
            });
        }
        if let Some(id) = &self.iroh_id {
            for app in crate::APPS {
                out.push(Entry {
                    kind: Kind::Iroh,
                    url: format!("hyperb://{id}/{app}"),
                });
            }
        }
        for name in self.wasm.keys() {
            out.push(Entry {
                kind: Kind::RemoteWasm,
                url: format!("hyperb://{host}/{name}.wasm"),
            });
        }
        out
    }

    /// The addresses, as lines for the terminal.
    pub fn listing(&self, host: &str) -> String {
        let entries = self.entries(host);
        let mut out = String::new();
        for kind in Kind::ALL {
            let urls: Vec<_> = entries.iter().filter(|e| e.kind == kind).collect();
            if urls.is_empty() {
                continue;
            }
            out.push_str(&format!("\n{}:\n", kind.title()));
            for e in urls {
                out.push_str(&format!("  {}\n", e.url));
            }
        }
        out
    }

    /// The page for people who open the server in an ordinary web browser.
    pub fn page(&self, host: &str) -> String {
        let entries = self.entries(host);
        let mut sections = String::new();
        for kind in Kind::ALL {
            let urls: Vec<_> = entries.iter().filter(|e| e.kind == kind).collect();
            if urls.is_empty() {
                continue;
            }
            sections.push_str(&format!("<h2>{}</h2>\n<ul>\n", kind.title()));
            for e in urls {
                sections.push_str(&format!("<li><code>{}</code></li>\n", escape(&e.url)));
            }
            sections.push_str("</ul>\n");
        }
        format!(
            r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>hyperb demo server</title>
<style>
  :root {{ color-scheme: light dark; --bg: #fff; --fg: #1d1d1f; --muted: #6b6b73; --chip: #f0f0f4; }}
  @media (prefers-color-scheme: dark) {{ :root {{ --bg: #17171a; --fg: #ececf0; --muted: #9a9aa5; --chip: #26262c; }} }}
  body {{ margin: 0 auto; max-width: 44rem; padding: 2rem 16px 4rem; background: var(--bg); color: var(--fg);
         font: 16px/1.5 system-ui, sans-serif; }}
  h1 {{ margin-bottom: 0; }}
  h2 {{ margin-top: 2rem; font-size: 1.05rem; }}
  p {{ color: var(--muted); }}
  ul {{ padding: 0; list-style: none; }}
  li {{ margin: 0.35rem 0; }}
  code {{ background: var(--chip); padding: 0.15rem 0.4rem; border-radius: 4px; overflow-wrap: anywhere; user-select: all; }}
  a {{ color: inherit; }}
</style>
</head>
<body>
<h1>hyperb demo server</h1>
<p>This server runs demo apps for <a href="{repo}">hyperb</a>, which are not web pages:
a normal web browser cannot open them. Copy an address below into the address bar
of a hyperb app (desktop, terminal, web or Android). The source and the apps
are at <a href="{repo}">{repo}</a>.</p>
{sections}</body>
</html>
"#,
            repo = REPO,
        )
    }
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyperb_core::address;
    use hyperb_core::identity::Identity;
    use hyperb_core::net::{Connection, Event};

    /// Opens `address` and returns what the app says to a hello.
    fn hello(address: &str) -> String {
        let url = address::normalize(address).unwrap();
        let conn = Connection::open(&url, &Identity::generate());
        let events = conn.events();
        assert_eq!(events.recv_blocking().unwrap(), Event::Open);
        conn.send("hello {}\n".into());
        match events.recv_blocking().unwrap() {
            Event::Data(line) => line,
            e => panic!("{e:?}"),
        }
    }

    /// Needs `just wasm-apps`; skipped without the modules.
    #[test]
    fn wasm_apps_say_hello() {
        let mut catalog = Catalog::default();
        catalog.load_wasm(Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../wasm-apps"
        )));
        for (name, wasm) in &catalog.wasm {
            let line = hello(&format!("hyperb://{}", wasm.path.display()));
            assert!(line.contains(' ') && line.ends_with('\n'), "{name}: {line}");
        }
    }

    /// Needs a running demoserver with wasm apps: `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn wasm_app_downloaded_from_the_server() {
        assert!(hello("hyperb://127.0.0.1:9001/qr.wasm").contains(' '));
    }

    #[test]
    fn page_lists_every_address() {
        let mut catalog = Catalog::default();
        catalog.iroh_id = Some("a".repeat(64));
        let page = catalog.page("localhost:9001");
        assert!(page.contains("hyperb://localhost:9001/restaurant"));
        assert!(page.contains(&format!("hyperb://{}/qr", "a".repeat(64))));
        assert!(page.contains(REPO));
    }
}
