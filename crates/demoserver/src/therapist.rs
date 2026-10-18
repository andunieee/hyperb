//! The therapist demo: book a weekly session, and be remembered.
//!
//! A client types their name, picks a weekday and an hour, and books the
//! standing weekly slot. The standing appointment and name are remembered by
//! the visitor's key, so the next visit (and every week after) opens with
//! their slot already waiting for them.

use super::store;
use hyperb_core::identity::PublicKey;
use hypereffect::{json, ui, Action, App, Handler, Map, Outbox, Value};
use std::sync::LazyLock;

/// The clients remembered between sessions. Read only through its methods;
/// `prune` trims forgotten entries.
pub(crate) static PATIENTS: LazyLock<store::Memory<Patient>> = LazyLock::new(store::Memory::new);

/// Weekdays for the standing slot.
const DAY_MENU: &[&str] = &["Monday", "Tuesday", "Wednesday", "Thursday", "Friday"];
/// Hours for the standing slot.
const HOUR_MENU: &[&str] = &[
    "09:00", "10:00", "11:00", "13:00", "14:00", "15:00", "16:00",
];

/// A remembered client.
#[derive(Clone, Default)]
pub(crate) struct Patient {
    /// Their name, as they typed it.
    name: String,
    /// The weekly slot, if one is booked.
    weekly: Option<Slot>,
}

#[derive(Clone)]
struct Slot {
    /// A string from `DAY_MENU`.
    day: String,
    /// A string from `HOUR_MENU`.
    hour: String,
}

#[derive(Default)]
pub struct Therapist;

fn key(signals: &Map<String, Value>) -> Option<PublicKey> {
    signals.get(super::KEY_SIGNAL)?.as_str()?.parse().ok()
}

/// A select menu in the component's shape.
fn menu(values: &[&str]) -> Vec<Value> {
    values
        .iter()
        .map(|v| json!({"value": v, "label": v}))
        .collect()
}

impl App for Therapist {
    fn hello(&mut self, out: &mut Outbox, signals: &Map<String, Value>) {
        let remembered = key(signals)
            .map(|k| PATIENTS.snapshot(&k))
            .unwrap_or_default();
        out.signal(json!({
            "client": remembered.name,
            "booked": remembered.weekly.is_some(),
            "menu": {
                "days": menu(DAY_MENU),
                "hours": menu(HOUR_MENU),
            },
            "form": {
                "name": remembered.name,
                "day": remembered.weekly.as_ref().map_or("Tuesday", |s| &s.day),
                "hour": remembered.weekly.as_ref().map_or("10:00", |s| &s.hour),
            },
            "feedback": {"kind": "info", "text": ""},
        }));
        out.element(ui::column("main").gap("large"));
        out.element(
            ui::text("title")
                .parent("main")
                .variant("h1")
                .text("Dr. Sol — Weekly Therapy Sessions"),
        );
        // Only shows once the server knows the client by name.
        out.element(
            ui::notice("welcome")
                .parent("main")
                .variant("success")
                .visible(json!({"cel": "s.client != ''"}))
                .text(json!({"cel": "'Welcome back, ' + s.client + '.'"})),
        );
        out.element(
            ui::card("booking")
                .parent("main")
                .title("Book your weekly session")
                .gap("medium"),
        );
        out.element(
            ui::input("name")
                .parent("booking")
                .label("Name")
                .bind("form.name")
                .placeholder("Who is coming to therapy?"),
        );
        out.element(
            ui::select("day")
                .parent("booking")
                .label("Day")
                .bind("form.day")
                .options(json!({"cel": "s.menu.days"})),
        );
        out.element(
            ui::select("hour")
                .parent("booking")
                .label("Hour")
                .bind("form.hour")
                .options(json!({"cel": "s.menu.hours"})),
        );
        out.element(
            ui::button("book")
                .parent("booking")
                .label("Book the weekly slot")
                .variant("primary")
                .on_press(Handler::action("book")),
        );
        // The standing appointment, if the client has one.
        out.element(
            ui::card("standing")
                .parent("main")
                .title("Your weekly session"),
        );
        out.element(
            ui::text("no_slot")
                .parent("standing")
                .visible(json!({"cel": "!s.booked"}))
                .text("No weekly session booked yet."),
        );
        out.element(
            ui::text("slot")
                .parent("standing")
                .visible(json!({"cel": "s.booked"}))
                .text(json!({
                    "cel": "'Every ' + s.form.day + ' at ' + s.form.hour + ', ' + s.client + '.'"
                })),
        );
        out.element(
            ui::button("end")
                .parent("standing")
                .label("End the standing appointment")
                .variant("failure")
                .visible(json!({"cel": "s.booked"}))
                .on_press(Handler::action("end")),
        );
        out.element(
            ui::notice("feedback")
                .parent("main")
                .variant(json!({"cel": "s.feedback.kind"}))
                .visible(json!({"cel": "s.feedback.text != ''"}))
                .text(json!({"cel": "s.feedback.text"})),
        );
        if !remembered.name.is_empty() {
            println!("a client is back: {}", remembered.name);
        }
    }

    fn action(&mut self, out: &mut Outbox, action: &Action) {
        let Some(key) = key(&action.signals) else {
            return;
        };
        match action.name.as_str() {
            "book" => {
                let text = |path: &str| {
                    action
                        .signal(path)
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .trim()
                        .to_owned()
                };
                let (name, day, hour) = (text("form.name"), text("form.day"), text("form.hour"));
                if name.is_empty() {
                    out.signal(json!({"feedback": {
                        "kind": "failure",
                        "text": "Please enter a name for the appointment.",
                    }}));
                    return;
                }
                PATIENTS.update(&key, |p| {
                    p.name = name.clone();
                    p.weekly = Some(Slot {
                        day: day.clone(),
                        hour: hour.clone(),
                    });
                });
                out.signal(json!({"client": name, "booked": true, "feedback": {
                    "kind": "success",
                    "text": format!("Every {day} at {hour} is yours."),
                }}));
                println!("{key} booked a weekly slot: {day}, {hour}");
            }
            "end" => {
                PATIENTS.update(&key, |p| p.weekly = None);
                out.signal(json!({"booked": false, "feedback": {
                    "kind": "info",
                    "text": "The standing appointment is ended.",
                }}));
                println!("{key} ended their weekly slot");
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyperb_core::identity::Identity;

    /// A client connecting as the transport sets the session up: the key is
    /// injected into the hello's signals.
    /// Triggers a handler with the key injected, as the transport does.
    fn trigger(
        l: &mut hypereffect::server::Loopback<Therapist>,
        key: PublicKey,
        id: &str,
        event: &str,
    ) -> bool {
        let Some(line) = l.client.trigger(id, event) else {
            return false;
        };
        l.server.receive(crate::with_key(&line, key).as_bytes());
        l.pump();
        true
    }

    fn connect(
        again: impl Into<Option<PublicKey>>,
    ) -> (PublicKey, hypereffect::server::Loopback<Therapist>) {
        let key = again
            .into()
            .unwrap_or_else(|| Identity::generate().public());
        let mut l = hypereffect::server::Loopback {
            client: hypereffect::client::Client::new(),
            server: hypereffect::Server::new(),
        };
        let hello = l.client.hello(Map::new());
        l.server.receive(crate::with_key(&hello, key).as_bytes());
        l.pump();
        (key, l)
    }

    #[test]
    fn first_visit_then_remembered() {
        let (key, mut l) = connect(None);
        assert_eq!(l.client.value("form.name"), Some(&json!("")));
        assert_eq!(l.client.value("booked"), Some(&json!(false)));
        // Book the weekly slot.
        l.write("form.name", json!("Robin"));
        l.write("form.day", json!("Thursday"));
        l.write("form.hour", json!("15:00"));
        assert!(trigger(&mut l, key, "book", "onPress"));
        assert_eq!(l.client.value("booked"), Some(&json!(true)));
        assert!(l.client.take_diagnostics().is_empty());
        // Return on a fresh session with the same key.
        drop(l);
        let mut l = connect(key).1;
        assert_eq!(l.client.value("form.day"), Some(&json!("Thursday")));
        assert_eq!(l.client.value("form.hour"), Some(&json!("15:00")));
        assert_eq!(
            l.client.attr("slot", "text"),
            Some(json!("Every Thursday at 15:00, Robin."))
        );
        assert_eq!(l.client.value("booked"), Some(&json!(true)));
        assert!(l.client.take_diagnostics().is_empty());
        // End it.
        assert!(trigger(&mut l, key, "end", "onPress"));
        assert_eq!(l.client.value("booked"), Some(&json!(false)));
        assert!(PATIENTS.snapshot(&key).weekly.is_none());
    }
}
