//! The restaurant demo: reserve a table, and be remembered.
//!
//! A guest types their name, picks a party size and a time slot, and books.
//! The booking (and the name) is remembered by the visitor's key, so when
//! they reconnect they are greeted and see their reservation again without
//! retyping anything.

use super::store;
use hyperb_core::identity::PublicKey;
use hypereffect::{App, Handler, Map, Outbox, Value, json, ui};
use std::collections::BTreeMap;
use std::sync::LazyLock;

/// The guests remembered between sessions. Read only through its methods;
/// `prune` trims forgotten entries.
pub(crate) static GUESTS: LazyLock<store::Memory<Guest>> = LazyLock::new(store::Memory::new);

/// Party sizes on the menu.
const SIZE_MENU: &[&str] = &["1", "2", "3", "4", "5", "6", "7", "8"];
/// Time slots for seating.
const SLOT_MENU: &[&str] = &[
    "17:00", "17:30", "18:00", "18:30", "19:00", "19:30", "20:00", "20:30", "21:00", "21:30",
];

/// A remembered regular.
#[derive(Clone, Default)]
pub(crate) struct Guest {
    /// Their name, as they typed it.
    name: String,
    /// The standing reservation, if any.
    table: Option<Table>,
}

#[derive(Clone)]
struct Table {
    /// Party size, as a string from `SIZE_MENU`.
    size: String,
    /// Time slot, as a string from `SLOT_MENU`.
    slot: String,
}

#[derive(Default)]
pub struct Restaurant;

fn key(signals: &Map<String, Value>) -> Option<PublicKey> {
    signals.get(super::KEY_SIGNAL)?.as_str()?.parse().ok()
}

fn param_key(params: &BTreeMap<String, String>) -> Option<PublicKey> {
    params.get(super::KEY_SIGNAL)?.parse().ok()
}

/// A select menu in the component's shape.
fn menu(values: &[&str]) -> Vec<Value> {
    values
        .iter()
        .map(|v| json!({"value": v, "label": v}))
        .collect()
}

impl App for Restaurant {
    fn hello(&mut self, out: &mut Outbox, params: &BTreeMap<String, String>) {
        let remembered = param_key(params)
            .map(|k| GUESTS.snapshot(&k))
            .unwrap_or_default();
        // Seed the form with the remembered profile.
        out.signal(json!({
            "guest": remembered.name,
            "booked": remembered.table.is_some(),
            "menu": {
                "sizes": menu(SIZE_MENU),
                "slots": menu(SLOT_MENU),
            },
            "form": {
                "name": remembered.name,
                "size": remembered.table.as_ref().map_or("2", |t| &t.size),
                "slot": remembered.table.as_ref().map_or("19:00", |t| &t.slot),
            },
            "feedback": {"kind": "info", "text": ""},
        }));
        out.element(ui::column("main").gap("large"));
        out.element(
            ui::text("title")
                .parent("main")
                .variant("h1")
                .text("Le Palmier — Table Reservations"),
        );
        // Only shows once the server knows the guest by name.
        out.element(
            ui::notice("welcome")
                .parent("main")
                .variant("success")
                .visible(json!({"cel": "s.guest != ''"}))
                .text(json!({"cel": "'Welcome back, ' + s.guest + '.'"})),
        );
        out.element(
            ui::card("booking")
                .parent("main")
                .title("Book a table")
                .gap("medium"),
        );
        out.element(
            ui::input("name")
                .parent("booking")
                .label("Name on the reservation")
                .bind("form.name")
                .placeholder("Who is the table for?"),
        );
        out.element(
            ui::select("size")
                .parent("booking")
                .label("Party size")
                .bind("form.size")
                .options(json!({"cel": "s.menu.sizes"})),
        );
        out.element(
            ui::select("slot")
                .parent("booking")
                .label("Time slot")
                .bind("form.slot")
                .options(json!({"cel": "s.menu.slots"})),
        );
        out.element(
            ui::button("reserve")
                .parent("booking")
                .label("Reserve")
                .variant("primary")
                .on_press(Handler::action("reserve")),
        );
        // What the guest has, plus the way out of it.
        out.element(ui::card("table").parent("main").title("Your reservation"));
        out.element(
            ui::text("no_table")
                .parent("table")
                .visible(json!({"cel": "!s.booked"}))
                .text("No table reserved yet."),
        );
        out.element(
            ui::row("details")
                .parent("table")
                .visible(json!({"cel": "s.booked"}))
                .gap("medium")
                .wrap(true),
        );
        out.element(
            ui::badge("table_size")
                .parent("details")
                .variant("info")
                .text(json!({"cel": "'party of ' + s.form.size"})),
        );
        out.element(
            ui::badge("table_slot")
                .parent("details")
                .variant("success")
                .text(json!({"cel": "'at ' + s.form.slot"})),
        );
        out.element(
            ui::button("cancel")
                .parent("table")
                .label("Cancel the reservation")
                .variant("failure")
                .visible(json!({"cel": "s.booked"}))
                .on_press(Handler::action("cancel")),
        );
        out.element(
            ui::notice("feedback")
                .parent("main")
                .variant(json!({"cel": "s.feedback.kind"}))
                .visible(json!({"cel": "s.feedback.text != ''"}))
                .text(json!({"cel": "s.feedback.text"})),
        );
        if !remembered.name.is_empty() {
            println!("a regular is back: {}", remembered.name);
        }
    }

    fn action(&mut self, out: &mut Outbox, action: &hypereffect::Action) {
        let Some(key) = key(&action.signals) else {
            return;
        };
        match action.name.as_str() {
            "reserve" => {
                let text = |path: &str| {
                    action
                        .signal(path)
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .trim()
                        .to_owned()
                };
                let (name, size, slot) = (text("form.name"), text("form.size"), text("form.slot"));
                if name.is_empty() {
                    out.signal(json!({"feedback": {
                        "kind": "failure",
                        "text": "Please enter a name for the reservation.",
                    }}));
                    return;
                }
                GUESTS.update(&key, |g| {
                    g.name = name.clone();
                    g.table = Some(Table {
                        size: size.clone(),
                        slot: slot.clone(),
                    });
                });
                out.signal(json!({"guest": name, "booked": true, "feedback": {
                    "kind": "success",
                    "text": format!("Table for {size} at {slot} reserved."),
                }}));
                println!("{key} reserved a table for {size} at {slot}");
            }
            "cancel" => {
                GUESTS.update(&key, |g| g.table = None);
                out.signal(json!({"booked": false, "feedback": {
                    "kind": "info",
                    "text": "The reservation is cancelled.",
                }}));
                println!("{key} cancelled their table");
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyperb_core::identity::Identity;

    /// Triggers a handler with the key injected, as the transport does.
    fn trigger(
        l: &mut hypereffect::server::Loopback<Restaurant>,
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

    fn connects(
        again: impl Into<Option<PublicKey>>,
    ) -> (PublicKey, hypereffect::server::Loopback<Restaurant>) {
        let key = again
            .into()
            .unwrap_or_else(|| Identity::generate().public());
        let mut l = hypereffect::server::Loopback {
            client: hypereffect::client::Client::new(),
            server: hypereffect::Server::new(),
        };
        // As the transport does: inject the key into the hello's params.
        let hello = l.client.hello();
        l.server.receive(crate::with_key(&hello, key).as_bytes());
        l.pump();
        (key, l)
    }

    #[test]
    fn first_visit_then_remembered() {
        let (key, mut l) = connects(None);
        // The form starts neutral.
        assert_eq!(l.client.value("form.name"), Some(&json!("")));
        assert_eq!(l.client.value("booked"), Some(&json!(false)));
        assert_eq!(l.client.attr("welcome", "visible"), Some(json!(false)));
        // Reserve.
        l.write("form.name", json!("Ann"));
        l.write("form.size", json!("4"));
        l.write("form.slot", json!("20:00"));
        assert!(trigger(&mut l, key, "reserve", "onPress"));
        assert_eq!(l.client.value("booked"), Some(&json!(true)));
        assert!(l.client.take_diagnostics().is_empty());
        // Come back on a fresh session with the same key.
        drop(l);
        let mut l = connects(key).1;
        assert_eq!(l.client.value("form.name"), Some(&json!("Ann")));
        assert_eq!(l.client.value("form.size"), Some(&json!("4")));
        assert_eq!(l.client.value("form.slot"), Some(&json!("20:00")));
        assert_eq!(l.client.value("booked"), Some(&json!(true)));
        assert_eq!(
            l.client.attr("welcome", "text"),
            Some(json!("Welcome back, Ann."))
        );
        assert!(l.client.take_diagnostics().is_empty());
        assert!(GUESTS.snapshot(&key).table.is_some());
        // Cancel it.
        assert!(trigger(&mut l, key, "cancel", "onPress"));
        assert_eq!(l.client.value("booked"), Some(&json!(false)));
        assert!(GUESTS.snapshot(&key).table.is_none());
    }

    #[test]
    fn empty_name_rejected() {
        let (_, mut l) = connects(None);
        assert!(trigger(
            &mut l,
            Identity::generate().public(),
            "reserve",
            "onPress"
        ));
        assert_eq!(l.client.value("booked"), Some(&json!(false)));
        assert_eq!(
            l.client.attr("feedback", "text"),
            Some(json!("Please enter a name for the reservation."))
        );
        assert!(l.client.take_diagnostics().is_empty());
    }
}
