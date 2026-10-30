//! The demo apps. They need nothing from a server, so `hyperb-wasmapps`
//! builds each of them as a wasm module.

pub mod billsplit;
pub mod chart;
pub mod common;
pub mod dbclient;
pub mod finance;
#[cfg(feature = "grammar")]
pub mod grammar;
pub mod img;
pub mod minecraft;
pub mod pomodoro;
pub mod qr;
pub mod restaurant;
pub mod saas;
pub mod smarthome;
pub mod textstats;
pub mod therapist;
