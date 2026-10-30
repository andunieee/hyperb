//! The therapist demo: book a weekly session.
//!
//! A client types their name, picks a weekday and an hour, and books the
//! standing weekly slot, which lasts as long as the session.

use hypereffect::{Action, App, Handler, Outbox, Value, json, ui};
use std::collections::BTreeMap;

/// Weekdays for the standing slot.
const DAY_MENU: &[&str] = &["Monday", "Tuesday", "Wednesday", "Thursday", "Friday"];
/// Hours for the standing slot.
const HOUR_MENU: &[&str] = &[
    "09:00", "10:00", "11:00", "13:00", "14:00", "15:00", "16:00",
];

#[derive(Default)]
pub struct Therapist;

/// A select menu in the component's shape.
fn menu(values: &[&str]) -> Vec<Value> {
    values
        .iter()
        .map(|v| json!({"value": v, "label": v}))
        .collect()
}

impl App for Therapist {
    fn hello(&mut self, out: &mut Outbox, _: &BTreeMap<String, String>) {
        out.signal(json!({
            "client": "",
            "booked": false,
            "menu": {
                "days": menu(DAY_MENU),
                "hours": menu(HOUR_MENU),
            },
            "form": {
                "name": "",
                "day": "Tuesday",
                "hour": "10:00",
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
    }

    fn action(&mut self, out: &mut Outbox, action: &Action) {
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
                out.signal(json!({"client": name, "booked": true, "feedback": {
                    "kind": "success",
                    "text": format!("Every {day} at {hour} is yours."),
                }}));
            }
            "end" => {
                out.signal(json!({"booked": false, "feedback": {
                    "kind": "info",
                    "text": "The standing appointment is ended.",
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

    #[test]
    fn book_then_end() {
        let mut l = Loopback::<Therapist>::new();
        l.pump();
        assert_eq!(l.client.value("form.name"), Some(&json!("")));
        assert_eq!(l.client.value("booked"), Some(&json!(false)));
        // Book the weekly slot.
        l.write("form.name", json!("Robin"));
        l.write("form.day", json!("Thursday"));
        l.write("form.hour", json!("15:00"));
        assert!(l.trigger("book", "onPress"));
        assert_eq!(l.client.value("booked"), Some(&json!(true)));
        assert_eq!(
            l.client.attr("slot", "text"),
            Some(json!("Every Thursday at 15:00, Robin."))
        );
        assert!(l.client.take_diagnostics().is_empty());
        // End it.
        assert!(l.trigger("end", "onPress"));
        assert_eq!(l.client.value("booked"), Some(&json!(false)));
    }
}
