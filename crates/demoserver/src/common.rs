//! The helpers of the demo apps, and the one that needs the session key.

pub use demoapps::common::*;
use hyperb_core::identity::PublicKey;
use hypereffect::{Map, Value};
use std::collections::BTreeMap;

/// The session key the transport injected into the hello's params, if any.
pub fn param_key(params: &BTreeMap<String, String>) -> Option<PublicKey> {
    params.get(crate::KEY_SIGNAL)?.parse().ok()
}

/// The session key the transport injected into the action's signals, if any.
pub fn key(signals: &Map<String, Value>) -> Option<PublicKey> {
    signals.get(crate::KEY_SIGNAL)?.as_str()?.parse().ok()
}
