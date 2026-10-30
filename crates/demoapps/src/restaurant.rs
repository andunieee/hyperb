//! The restaurant demo: reserve a table.
//!
//! A guest types their name, picks a party size and a time slot, and books.
//! The booking lasts as long as the session.

use hypereffect::{App, Handler, Outbox, Value, json, ui};
use std::collections::BTreeMap;

/// Party sizes on the menu.
const SIZE_MENU: &[&str] = &["1", "2", "3", "4", "5", "6", "7", "8"];
/// Time slots for seating.
const SLOT_MENU: &[&str] = &[
    "17:00", "17:30", "18:00", "18:30", "19:00", "19:30", "20:00", "20:30", "21:00", "21:30",
];

#[derive(Default)]
pub struct Restaurant;

/// A select menu in the component's shape.
fn menu(values: &[&str]) -> Vec<Value> {
    values
        .iter()
        .map(|v| json!({"value": v, "label": v}))
        .collect()
}

impl App for Restaurant {
    fn hello(&mut self, out: &mut Outbox, _: &BTreeMap<String, String>) {
        out.signal(json!({
            "guest": "",
            "booked": false,
            "menu": {
                "sizes": menu(SIZE_MENU),
                "slots": menu(SLOT_MENU),
            },
            "form": {
                "name": "",
                "size": "2",
                "slot": "19:00",
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
    }

    fn action(&mut self, out: &mut Outbox, action: &hypereffect::Action) {
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
                out.signal(json!({"guest": name, "booked": true, "feedback": {
                    "kind": "success",
                    "text": format!("Table for {size} at {slot} reserved."),
                }}));
            }
            "cancel" => {
                out.signal(json!({"booked": false, "feedback": {
                    "kind": "info",
                    "text": "The reservation is cancelled.",
                }}));
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hypereffect::server::Loopback;

    fn connect() -> Loopback<Restaurant> {
        let mut l = Loopback::new();
        l.pump();
        l
    }

    #[test]
    fn reserve_then_cancel() {
        let mut l = connect();
        // The form starts neutral.
        assert_eq!(l.client.value("form.name"), Some(&json!("")));
        assert_eq!(l.client.value("booked"), Some(&json!(false)));
        // Reserve.
        l.write("form.name", json!("Ann"));
        l.write("form.size", json!("4"));
        l.write("form.slot", json!("20:00"));
        assert!(l.trigger("reserve", "onPress"));
        assert_eq!(l.client.value("booked"), Some(&json!(true)));
        assert_eq!(l.client.value("guest"), Some(&json!("Ann")));
        assert!(l.client.take_diagnostics().is_empty());
        // Cancel it.
        assert!(l.trigger("cancel", "onPress"));
        assert_eq!(l.client.value("booked"), Some(&json!(false)));
    }

    #[test]
    fn empty_name_rejected() {
        let mut l = connect();
        assert!(l.trigger("reserve", "onPress"));
        assert_eq!(l.client.value("booked"), Some(&json!(false)));
        assert_eq!(
            l.client.attr("feedback", "text"),
            Some(json!("Please enter a name for the reservation."))
        );
        assert!(l.client.take_diagnostics().is_empty());
    }
}
