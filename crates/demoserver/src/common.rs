//! The helpers of the demo apps, and the one that needs the session key.

pub use demoapps::common::*;
use hyperb_core::identity::PublicKey;
use hypereffect::{Map, Value};

/// The session key the transport injected into the signals, if any.
pub fn key(signals: &Map<String, Value>) -> Option<PublicKey> {
    signals.get(crate::KEY_SIGNAL)?.as_str()?.parse().ok()
}
