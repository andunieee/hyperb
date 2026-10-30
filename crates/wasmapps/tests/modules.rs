//! Runs the modules `just wasm-apps` built, as the renderers do. Skipped
//! without them.

use hyperb_core::address;
use hyperb_core::identity::Identity;
use hyperb_core::net::{Connection, Event};
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn modules() -> Vec<PathBuf> {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../wasm-apps");
    let Ok(files) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    files
        .flatten()
        .map(|f| f.path())
        .filter(|p| p.extension().is_some_and(|e| e == "wasm"))
        .collect()
}

fn open(path: &PathBuf) -> Connection {
    let path = std::fs::canonicalize(path).unwrap();
    let url = address::normalize(&format!("hyperb://{}", path.display())).unwrap();
    Connection::open(&url, &Identity::generate())
}

#[test]
fn every_module_says_hello() {
    for path in modules() {
        let conn = open(&path);
        let events = conn.events();
        assert_eq!(events.recv_blocking().unwrap(), Event::Open);
        conn.send("hello {}\n".into());
        match events.recv_blocking().unwrap() {
            Event::Data(line) => assert!(line.contains(' '), "{}: {line}", path.display()),
            e => panic!("{}: {e:?}", path.display()),
        }
    }
}

/// The smart home ticks every second, through `wake_in` and `wake`.
#[test]
fn smarthome_ticks() {
    let Some(path) = modules()
        .into_iter()
        .find(|p| p.file_stem().is_some_and(|n| n == "smarthome"))
    else {
        return;
    };
    let conn = open(&path);
    let events = conn.events();
    assert_eq!(events.recv_blocking().unwrap(), Event::Open);
    conn.send("hello {}\n".into());
    // Everything the hello made arrives at once; what follows is a tick.
    let start = Instant::now();
    let mut ticked = false;
    while start.elapsed() < Duration::from_secs(5) {
        match events.recv_blocking().unwrap() {
            Event::Data(_) if start.elapsed() > Duration::from_millis(500) => {
                ticked = true;
                break;
            }
            Event::Data(_) => {}
            e => panic!("{e:?}"),
        }
    }
    assert!(ticked);
}
