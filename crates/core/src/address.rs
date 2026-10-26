//! What the user types in the URL bar, turned into a session URL. See
//! `SPEC.md` for the `hyperb://` scheme.
//!
//! The result is a [`Url`] whose scheme names the transport, which is what
//! [`crate::net::Connection::open`] dispatches on:
//!
//! - `http` / `https`: HTTP with SSE.
//! - `iroh`: an iroh session; the host is the endpoint id (64 hex chars).
//! - `wasm`: a local wasm app; the path is the file, the fragment the route.
//!
//! The transport scheme is internal. Address bars display the user-facing
//! `hyperb://` form instead, produced by [`hyperb`]; what users type in may
//! always be fed back in, and normalize accepts transports schemes too.

use url::{Host, Url};

pub fn normalize(input: &str) -> Result<Url, String> {
    let input = input.trim();
    if input.is_empty() {
        return Err("empty address".into());
    }
    let (scheme, rest) = match input.split_once("://") {
        Some((scheme, rest)) => (Some(scheme.to_ascii_lowercase()), rest),
        None => (None, input),
    };
    let forced = match scheme.as_deref() {
        // `hyperb` is the user-facing scheme, and `web+hyperb` its name where
        // web browsers only let pages handle `web+` schemes; `iroh` and
        // `wasm` are accepted already normalized (a reconnect or a reshared
        // display address).
        None | Some("hyperb") | Some("web+hyperb") | Some("iroh") | Some("wasm") => None,
        Some("http") | Some("hyperb+http") => Some("http"),
        Some("https") | Some("hyperb+https") => Some("https"),
        Some(other) => return Err(format!("unsupported scheme '{other}'")),
    };

    // A local file: `hyperb:///path/app.wasm`, or just an absolute path.
    if forced.is_none() && (rest.starts_with('/') || scheme.is_none() && rest.starts_with("~/")) {
        return wasm(rest);
    }
    if rest.starts_with('/') {
        return Err("missing host".into());
    }
    let url = parse(&format!("http://{rest}"))?;
    let host = match url.host() {
        Some(host) => host,
        None => return Err("missing host".into()),
    };
    if let Host::Domain(d) = &host {
        if is_endpoint_id(d) {
            if forced.is_some() {
                return Err("an iroh address cannot use http or https".into());
            }
            if rest
                .split(['/', '?', '#'])
                .next()
                .is_some_and(|a| a.contains(':'))
            {
                return Err("an iroh address has no port".into());
            }
            let mut out = format!("iroh://{}{}", d.to_ascii_lowercase(), url.path());
            if let Some(q) = url.query() {
                out.push_str(&format!("?{q}"));
            }
            return parse(&out);
        }
    }
    let scheme = forced.unwrap_or(if is_plain_host(&host) {
        "http"
    } else {
        "https"
    });
    parse(&format!("{scheme}://{rest}"))
}

/// [`normalize`] for an address that comes from a link (another app, a web
/// page, the system's URL handler) rather than from the user: local files are
/// refused (`SPEC.md`, section 1.5), and so is input without a scheme.
pub fn from_link(input: &str) -> Result<Url, String> {
    if !input.contains("://") {
        return Err("a link must have a scheme".into());
    }
    let url = normalize(input)?;
    if kind(&url) == Kind::LocalWasm {
        return Err("links cannot open local apps".into());
    }
    Ok(url)
}

fn wasm(path: &str) -> Result<Url, String> {
    let (path, route) = match path.split_once('#') {
        Some((p, r)) => (p, Some(r)),
        None => (path, None),
    };
    let path = match path.strip_prefix("~/") {
        Some(rest) => {
            let home = std::env::var("HOME").map_err(|_| "no home directory")?;
            format!("{}/{rest}", home.trim_end_matches('/'))
        }
        None => path.to_string(),
    };
    if path.ends_with('/') {
        return Err("a local app must be a file".into());
    }
    let mut url = Url::parse("wasm:///").map_err(|e| e.to_string())?;
    url.set_path(&path);
    url.set_fragment(route);
    Ok(url)
}

/// A short label for a tab: the host, with the port and path if it has them.
pub fn label(url: &Url) -> String {
    match url.scheme() {
        "wasm" => url
            .path_segments()
            .and_then(|mut s| s.next_back())
            .unwrap_or_default()
            .to_string(),
        "iroh" => {
            let id = url.host_str().unwrap_or_default();
            let path = url.path().trim_end_matches('/');
            format!("{}…{path}", &id[..id.len().min(8)])
        }
        _ => {
            let host = url.host_str().unwrap_or_default();
            let port = url.port().map(|p| format!(":{p}")).unwrap_or_default();
            let path = url.path().trim_end_matches('/');
            format!("{host}{port}{path}")
        }
    }
}

/// The user-facing `hyperb://` form of a normalized address, for display.
///
/// Address bars show this, not the normalized URL: `hyperb://` is the scheme
/// users type and share, `http`, `iroh` and `wasm` are internal transports.
/// Displays keep the scheme the user chose where `hyperb://` would resolve it
/// differently (`hyperb+http` on an https-defaulting host and vice versa).
/// Feeding the result into [`normalize`] gives back `url`.
pub fn hyperb(url: &Url) -> String {
    let query = url.query().map(|q| format!("?{q}")).unwrap_or_default();
    match url.scheme() {
        "iroh" => {
            let id = url.host_str().unwrap_or_default();
            format!("hyperb://{id}{}{query}", url.path())
        }
        "wasm" => {
            let mut out = format!("hyperb://{}", url.path());
            if let Some(route) = url.fragment() {
                out.push_str(&format!("#{route}"));
            }
            out
        }
        scheme => {
            let prefix = match (scheme, url.host()) {
                ("http", Some(h)) if !is_plain_host(&h) => "hyperb+http",
                ("https", Some(h)) if is_plain_host(&h) => "hyperb+https",
                _ => "hyperb",
            };
            let rest = match url.as_str().split_once("://") {
                Some((_, r)) => r,
                None => "",
            };
            format!("{prefix}://{rest}")
        }
    }
}

/// The file of a `wasm` URL, percent-decoded.
pub fn wasm_path(url: &Url) -> Option<std::path::PathBuf> {
    if url.scheme() != "wasm" {
        return None;
    }
    Some(std::path::PathBuf::from(
        String::from_utf8(percent_decode(url.path())).ok()?,
    ))
}

/// The kind of session a normalized address opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Http,
    Iroh,
    /// A wasm app read from a local file.
    LocalWasm,
    /// A wasm app downloaded over http(s) and run locally.
    RemoteWasm,
}

pub fn kind(url: &Url) -> Kind {
    match url.scheme() {
        "iroh" => Kind::Iroh,
        "wasm" => Kind::LocalWasm,
        _ if url.path().to_ascii_lowercase().ends_with(".wasm") => Kind::RemoteWasm,
        _ => Kind::Http,
    }
}

/// Whether the UI can show the session as secure: nobody on the network can
/// read or alter it, and no code from a remote party runs on this machine.
/// That is `https` apps, iroh, and wasm apps from local files. A downloaded
/// wasm app is never secure, even over https, because it is foreign code.
pub fn is_secure(url: &Url) -> bool {
    match kind(url) {
        Kind::Iroh | Kind::LocalWasm => true,
        Kind::Http => url.scheme() == "https",
        Kind::RemoteWasm => false,
    }
}

/// The prefix of the ALPN of iroh sessions.
pub const IROH_ALPN: &str = "hyperb/0";

/// The ALPN an iroh session at `url` offers: [`IROH_ALPN`] followed by the
/// path and query, e.g. `hyperb/0/restaurant`. The root path adds nothing.
pub fn iroh_alpn(url: &Url) -> Result<Vec<u8>, String> {
    let path = if url.path() == "/" { "" } else { url.path() };
    let query = url.query().map(|q| format!("?{q}")).unwrap_or_default();
    let alpn = format!("{IROH_ALPN}{path}{query}").into_bytes();
    if alpn.len() > 255 {
        return Err("the address is too long for an iroh session".into());
    }
    Ok(alpn)
}

/// The path and query (starting with `/`) an ALPN offered by
/// [`iroh_alpn`] stands for, or `None` if it is not a hyperb ALPN.
pub fn iroh_alpn_path(alpn: &[u8]) -> Option<String> {
    let rest = std::str::from_utf8(alpn).ok()?.strip_prefix(IROH_ALPN)?;
    match rest.chars().next() {
        None => Some("/".into()),
        Some('/') => Some(rest.into()),
        Some('?') => Some(format!("/{rest}")),
        Some(_) => None,
    }
}

fn percent_decode(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    out
}

/// An iroh endpoint id: an ed25519 public key as 64 hex characters. A DNS
/// label has at most 63, so this is never a hostname.
fn is_endpoint_id(host: &str) -> bool {
    host.len() == 64 && host.bytes().all(|b| b.is_ascii_hexdigit())
}

fn parse(s: &str) -> Result<Url, String> {
    Url::parse(s).map_err(|e| e.to_string())
}

/// Hosts that default to http: `localhost`, bare IP addresses and onion
/// services.
fn is_plain_host(host: &Host<&str>) -> bool {
    match host {
        Host::Domain(d) => {
            let d = d.trim_end_matches('.');
            d == "localhost" || d.ends_with(".localhost") || d.ends_with(".onion")
        }
        Host::Ipv4(_) | Host::Ipv6(_) => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789ABCDEF";

    fn n(s: &str) -> String {
        normalize(s).unwrap().to_string()
    }

    #[test]
    fn schemes() {
        assert_eq!(n("hyperb://example.com/a"), "https://example.com/a");
        assert_eq!(n("HYPERB://example.com"), "https://example.com/");
        assert_eq!(n("http://example.com/a"), "http://example.com/a");
        assert_eq!(n("https://example.com"), "https://example.com/");
        assert_eq!(n("hyperb+http://example.com"), "http://example.com/");
        assert_eq!(
            n("hyperb+https://localhost:9001"),
            "https://localhost:9001/"
        );
        assert_eq!(n("web+hyperb://example.com/a"), "https://example.com/a");
        assert_eq!(n("Web+Hyperb://localhost:9001"), "http://localhost:9001/");
        assert!(normalize("web+hyperb+http://example.com").is_err());
        assert!(normalize("ws://example.com").is_err());
        assert!(normalize("ftp://example.com").is_err());
        assert!(normalize("  ").is_err());
    }

    #[test]
    fn default_scheme_depends_on_host() {
        for (input, want) in [
            (
                "localhost:9001/restaurant",
                "http://localhost:9001/restaurant",
            ),
            ("hyperb://localhost:9001", "http://localhost:9001/"),
            ("hyperb://127.0.0.1:9001", "http://127.0.0.1:9001/"),
            ("hyperb://192.168.1.20/psu", "http://192.168.1.20/psu"),
            ("hyperb://8.8.8.8", "http://8.8.8.8/"),
            ("hyperb://[::1]:8080", "http://[::1]:8080/"),
            ("hyperb://abc.onion/x", "http://abc.onion/x"),
            (
                "hyperb://hypereffect.dev/demo",
                "https://hypereffect.dev/demo",
            ),
            ("hypereffect.dev/demo", "https://hypereffect.dev/demo"),
            ("hyperb://printer.local", "https://printer.local/"),
        ] {
            assert_eq!(n(input), want, "{input}");
        }
    }

    #[test]
    fn iroh() {
        let id = ID.to_ascii_lowercase();
        assert_eq!(n(&format!("hyperb://{ID}")), format!("iroh://{id}/"));
        assert_eq!(
            n(&format!("hyperb://{id}/a/b?x=1")),
            format!("iroh://{id}/a/b?x=1")
        );
        assert_eq!(n(&id), format!("iroh://{id}/"));
        assert!(normalize(&format!("hyperb+https://{id}")).is_err());
        assert!(normalize(&format!("hyperb://{id}:80")).is_err());
        // 63 hex chars is a valid hostname.
        assert_eq!(
            n(&format!("hyperb://{}", &id[1..])),
            format!("https://{}/", &id[1..])
        );
    }

    #[test]
    fn wasm() {
        assert_eq!(n("hyperb:///opt/apps/a.wasm"), "wasm:///opt/apps/a.wasm");
        assert_eq!(n("/opt/apps/a.wasm#/menu"), "wasm:///opt/apps/a.wasm#/menu");
        let url = normalize("hyperb:///opt/my apps/a.wasm#/x").unwrap();
        assert_eq!(
            wasm_path(&url).unwrap().to_str(),
            Some("/opt/my apps/a.wasm")
        );
        assert_eq!(url.fragment(), Some("/x"));
        assert!(normalize("hyperb:///opt/apps/").is_err());
        assert!(normalize("hyperb+http:///opt/a.wasm").is_err());
        assert_eq!(wasm_path(&normalize("localhost").unwrap()), None);
    }

    #[test]
    fn links() {
        assert_eq!(
            from_link("web+hyperb://example.com/a").unwrap().as_str(),
            "https://example.com/a"
        );
        assert_eq!(
            from_link(&format!("hyperb://{ID}/x")).unwrap().scheme(),
            "iroh"
        );
        assert!(from_link("hyperb://example.com/a.wasm").is_ok());
        assert!(from_link("hyperb:///opt/a.wasm").is_err());
        assert!(from_link("web+hyperb:///opt/a.wasm").is_err());
        assert!(from_link("wasm:///opt/a.wasm").is_err());
        assert!(from_link("/opt/a.wasm").is_err());
        assert!(from_link("~/a.wasm").is_err());
    }

    #[test]
    fn labels() {
        let l = |s: &str| label(&normalize(s).unwrap());
        assert_eq!(l("localhost:9001/x"), "localhost:9001/x");
        assert_eq!(l("example.com/"), "example.com");
        assert_eq!(l("example.com/a/b/"), "example.com/a/b");
        assert_eq!(l(&format!("hyperb://{ID}/menu")), "01234567…/menu");
        assert_eq!(l("hyperb:///opt/apps/a.wasm"), "a.wasm");
    }

    #[test]
    fn display() {
        let d = |s: &str| hyperb(&normalize(s).unwrap());
        assert_eq!(d("https://example.com/a"), "hyperb://example.com/a");
        assert_eq!(d("hyperb://localhost:9001"), "hyperb://localhost:9001/");
        assert_eq!(d("http://example.com"), "hyperb+http://example.com/");
        assert_eq!(
            d("hyperb://localhost:9001/a.wasm?x=1"),
            "hyperb://localhost:9001/a.wasm?x=1"
        );
        let id = ID.to_ascii_lowercase();
        assert_eq!(
            d(&format!("hyperb://{ID}/menu")),
            format!("hyperb://{id}/menu")
        );
        assert_eq!(
            d("/opt/apps/a.wasm#/menu"),
            "hyperb:///opt/apps/a.wasm#/menu"
        );
        // Display addresses round-trip through normalize, and so do the
        // transport schemes, which normalize also accepts on input.
        for s in [
            "hyperb://example.com",
            "https://example.com/a",
            "http://localhost:9001/x",
            "http://example.com/x?a=1",
            "hyperb+http://example.com/x",
            "https://localhost:9001",
            &format!("hyperb://{ID}/menu?x=1"),
            "wasm:///opt/apps/a.wasm",
            "wasm:///opt/apps/a.wasm#/menu",
        ] {
            let url = normalize(s).unwrap();
            assert_eq!(normalize(&hyperb(&url)), Ok(url.clone()), "{s}");
            assert_eq!(normalize(url.as_str()).unwrap(), url, "{s}");
        }
    }

    #[test]
    fn kinds_and_alpn() {
        let k = |s: &str| kind(&normalize(s).unwrap());
        assert_eq!(k("example.com/app"), Kind::Http);
        assert_eq!(k("example.com/app.WASM#/x"), Kind::RemoteWasm);
        assert_eq!(k("localhost:80/a.wasm"), Kind::RemoteWasm);
        assert_eq!(k(&format!("hyperb://{ID}")), Kind::Iroh);
        assert_eq!(k("/a.wasm"), Kind::LocalWasm);
        let alpn = |p: &str| iroh_alpn(&normalize(&format!("hyperb://{ID}{p}")).unwrap()).unwrap();
        assert_eq!(alpn(""), b"hyperb/0");
        assert_eq!(alpn("/restaurant"), b"hyperb/0/restaurant");
        assert_eq!(alpn("/?a=1"), b"hyperb/0?a=1");
        assert_eq!(iroh_alpn_path(b"hyperb/0").as_deref(), Some("/"));
        assert_eq!(iroh_alpn_path(b"hyperb/0/a/b?x").as_deref(), Some("/a/b?x"));
        assert_eq!(iroh_alpn_path(b"hyperb/0?a=1").as_deref(), Some("/?a=1"));
        assert_eq!(iroh_alpn_path(b"hyperb/01"), None);
        assert_eq!(iroh_alpn_path(b"other"), None);
        let long = format!("hyperb://{ID}/{}", "a".repeat(300));
        assert!(iroh_alpn(&normalize(&long).unwrap()).is_err());
    }

    #[test]
    fn security() {
        assert!(!is_secure(&normalize("example.com/a.wasm").unwrap()));
        assert!(!is_secure(
            &normalize("hyperb+http://localhost/a.wasm").unwrap()
        ));
        assert!(is_secure(&normalize("example.com").unwrap()));
        assert!(is_secure(&normalize(ID).unwrap()));
        assert!(!is_secure(&normalize("localhost").unwrap()));
        assert!(is_secure(&normalize("/a.wasm").unwrap()));
    }
}
