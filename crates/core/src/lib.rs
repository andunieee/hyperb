//! Shared logic used by desktop (gtk4), terminal (ratatui), web (leptos),
//! android (kotlin via JNI).
//!
//! - [`address`]: turning what the user typed in the URL bar into a session URL
//!   (`hyperb://`, see `SPEC.md`).
//! - [`identity`]: the renderer's ed25519 keypair, presented to servers.
//! - [`http`]: the HTTP transport's protocol, shared with servers; [`sse`]
//!   parses its event stream.
//! - [`net`] (feature `net`): connections over the transports.
//! - [`hypereffect`] is re-exported; its `client::Client` is the renderer core.

pub mod address;
pub mod http;
pub mod identity;
#[cfg(feature = "net")]
pub mod net;
pub mod sse;

pub use hypereffect;

pub fn greeting(name: &str) -> String {
    format!("Hello, {name} from hyperb-core!")
}

pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn greeting_works() {
        assert!(greeting("world").contains("hyperb-core"));
    }
}
