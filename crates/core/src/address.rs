//! What the user types in the URL bar, turned into a server URL.
//!
//! - `http://` and `https://` are used as they are.
//! - Without a scheme, local hosts (`localhost`, loopback and private
//!   addresses) get `http://` and everything else gets `https://`.

use std::net::IpAddr;
use url::{Host, Url};

pub fn normalize(input: &str) -> Result<Url, String> {
    let input = input.trim();
    if input.is_empty() {
        return Err("empty address".into());
    }
    let url = if let Some((scheme, rest)) = input.split_once("://") {
        let scheme = match scheme.to_ascii_lowercase().as_str() {
            "http" => "http",
            "https" => "https",
            other => return Err(format!("unsupported scheme '{other}'")),
        };
        parse(&format!("{scheme}://{rest}"))?
    } else {
        let url = parse(&format!("http://{input}"))?;
        if is_local(&url) {
            url
        } else {
            parse(&format!("https://{input}"))?
        }
    };
    if url.host().is_none() {
        return Err("missing host".into());
    }
    Ok(url)
}

/// A short label for a tab: the host, with the port and path if it has them.
pub fn label(url: &Url) -> String {
    let host = url.host_str().unwrap_or_default();
    let port = url.port().map(|p| format!(":{p}")).unwrap_or_default();
    let path = url.path().trim_end_matches('/');
    format!("{host}{port}{path}")
}

fn parse(s: &str) -> Result<Url, String> {
    Url::parse(s).map_err(|e| e.to_string())
}

fn is_local(url: &Url) -> bool {
    match url.host() {
        Some(Host::Domain(d)) => {
            d == "localhost" || d.ends_with(".localhost") || d.ends_with(".local")
        }
        Some(Host::Ipv4(ip)) => is_local_ip(IpAddr::V4(ip)),
        Some(Host::Ipv6(ip)) => is_local_ip(IpAddr::V6(ip)),
        None => false,
    }
}

fn is_local_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            ip.is_loopback() || ip.is_private() || ip.is_link_local() || ip.is_unspecified()
        }
        IpAddr::V6(ip) => ip.is_loopback() || ip.is_unspecified(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(s: &str) -> String {
        normalize(s).unwrap().to_string()
    }

    #[test]
    fn schemes() {
        assert_eq!(n("http://example.com/a"), "http://example.com/a");
        assert_eq!(n("https://example.com"), "https://example.com/");
        assert_eq!(n("HTTP://example.com"), "http://example.com/");
        assert!(normalize("ws://example.com").is_err());
        assert!(normalize("ftp://example.com").is_err());
        assert!(normalize("  ").is_err());
    }

    #[test]
    fn default_scheme_depends_on_host() {
        assert_eq!(
            n("localhost:9001/restaurant"),
            "http://localhost:9001/restaurant"
        );
        assert_eq!(n("127.0.0.1:9001"), "http://127.0.0.1:9001/");
        assert_eq!(n("192.168.1.20/psu"), "http://192.168.1.20/psu");
        assert_eq!(n("[::1]:8080"), "http://[::1]:8080/");
        assert_eq!(n("hypereffect.dev/demo"), "https://hypereffect.dev/demo");
    }

    #[test]
    fn labels() {
        assert_eq!(
            label(&normalize("localhost:9001/x").unwrap()),
            "localhost:9001/x"
        );
        assert_eq!(label(&normalize("example.com/").unwrap()), "example.com");
        assert_eq!(
            label(&normalize("example.com/a/b/").unwrap()),
            "example.com/a/b"
        );
    }
}
