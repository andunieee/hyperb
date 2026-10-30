//! The bill splitter demo: who owes what.
//!
//! The total goes in once. Each person has a name, a share and what they
//! have paid so far. A share is a percent (`50%`), an amount (`20`), or blank:
//! blank shares split what the others leave over, equally. Everything is
//! recomputed on each edit and shown in the person's own line, along with how
//! much of the bill is still unpaid.

use super::common::{money, parse_cents};
use hypereffect::{Action, App, Handler, Map, Outbox, Value, cel, json, ui};
use std::collections::BTreeMap;

#[derive(Default)]
pub struct BillSplit {
    /// The people on the bill, in order, by id.
    ids: Vec<u32>,
    next: u32,
}

/// One person's inputs, as typed.
struct Entry {
    share: String,
    paid: String,
}

#[derive(Debug, PartialEq)]
struct Line {
    /// What the person should pay, in cents.
    share: i64,
    paid: i64,
    share_error: String,
    paid_error: String,
}

#[derive(Debug, PartialEq)]
struct Report {
    total: i64,
    total_error: String,
    lines: Vec<Line>,
    /// Total minus everything paid: negative when overpaid.
    left: i64,
    /// A problem with the shares as a whole, if any.
    note: String,
}

enum Share {
    Fixed(i64),
    Blank,
}

fn split(total: &str, entries: &[Entry]) -> Report {
    let total_text = total.trim();
    let (total, total_error) = match (total_text.is_empty(), parse_cents(total_text)) {
        (true, _) => (0, String::new()),
        (false, Some(c)) if c >= 0 => (c, String::new()),
        _ => (0, "Not a valid amount.".to_owned()),
    };
    let mut lines = Vec::with_capacity(entries.len());
    let mut shares = Vec::with_capacity(entries.len());
    for e in entries {
        let (share_text, paid_text) = (e.share.trim(), e.paid.trim());
        let mut share_error = String::new();
        let share = if share_text.is_empty() {
            Share::Blank
        } else if let Some(pct) = share_text.strip_suffix('%') {
            match pct.trim().replace(',', ".").parse::<f64>() {
                Ok(p) if p.is_finite() && p >= 0.0 => {
                    Share::Fixed((total as f64 * p / 100.0).round() as i64)
                }
                _ => {
                    share_error = "Not a valid percent.".into();
                    Share::Fixed(0)
                }
            }
        } else {
            match parse_cents(share_text) {
                Some(c) if c >= 0 => Share::Fixed(c),
                _ => {
                    share_error = "Use a percent, an amount, or leave blank.".into();
                    Share::Fixed(0)
                }
            }
        };
        let (paid, paid_error) = match (paid_text.is_empty(), parse_cents(paid_text)) {
            (true, _) => (0, String::new()),
            (false, Some(c)) if c >= 0 => (c, String::new()),
            _ => (0, "Not a valid amount.".to_owned()),
        };
        shares.push(share);
        lines.push(Line {
            share: 0,
            paid,
            share_error,
            paid_error,
        });
    }
    // Blank shares divide what the fixed ones leave over, to the cent.
    let fixed: i64 = shares
        .iter()
        .filter_map(|s| match s {
            Share::Fixed(c) => Some(*c),
            Share::Blank => None,
        })
        .sum();
    let leftover = total - fixed;
    let blanks = shares.iter().filter(|s| matches!(s, Share::Blank)).count() as i64;
    let pool = leftover.max(0);
    let mut seen = 0;
    for (line, share) in lines.iter_mut().zip(&shares) {
        line.share = match share {
            Share::Fixed(c) => *c,
            Share::Blank => {
                let extra = i64::from(seen < pool % blanks);
                seen += 1;
                pool / blanks + extra
            }
        };
    }
    let note = if leftover < 0 {
        format!("The shares exceed the total by {}.", money(-leftover))
    } else if blanks == 0 && leftover > 0 && !entries.is_empty() {
        format!(
            "{} of the total is not assigned to anyone: leave a share blank to split it.",
            money(leftover)
        )
    } else {
        String::new()
    };
    let paid: i64 = lines.iter().map(|l| l.paid).sum();
    Report {
        total,
        total_error,
        lines,
        left: total - paid,
        note,
    }
}

/// The text and the badge variant for what a person still owes.
fn status(owes: i64) -> (String, &'static str) {
    match owes {
        0 => ("settled".into(), "success"),
        o if o > 0 => (format!("owes {}", money(o)), "warning"),
        o => (format!("overpaid by {}", money(-o)), "info"),
    }
}

impl BillSplit {
    fn person(&mut self, out: &mut Outbox) {
        self.next += 1;
        let n = self.next;
        self.ids.push(n);
        let (row, field) = (format!("row_{n}"), |what: &str| format!("{what}_{n}"));
        let path = |what: &str| format!("people.p{n}.{what}");
        out.signal(json!({
            "people": {format!("p{n}"): {"name": "", "share": "", "paid": ""}},
            "calc": {format!("p{n}"): {"due": "", "status": "", "kind": "neutral"}},
            "err": {format!("p{n}"): {"share": "", "paid": ""}},
        }));
        out.element(
            ui::row(&row)
                .parent("people")
                .gap("medium")
                .align("end")
                .wrap(true),
        );
        out.element(
            ui::input(field("name"))
                .parent(&row)
                .label("Name")
                .placeholder("Who?")
                .bind(path("name")),
        );
        out.element(
            ui::input(field("share"))
                .parent(&row)
                .label("Has to pay")
                .placeholder("50%, 20 or blank")
                .bind(path("share"))
                .error(cel(format!("s.err.p{n}.share")))
                .on_change(Handler::action("edit")),
        );
        out.element(
            ui::input(field("paid"))
                .parent(&row)
                .label("Has paid")
                .placeholder("0")
                .bind(path("paid"))
                .error(cel(format!("s.err.p{n}.paid")))
                .on_change(Handler::action("edit")),
        );
        out.element(
            ui::text(field("due"))
                .parent(&row)
                .variant("caption")
                .text(cel(format!("s.calc.p{n}.due"))),
        );
        out.element(
            ui::badge(field("status"))
                .parent(&row)
                .variant(cel(format!("s.calc.p{n}.kind")))
                .text(cel(format!("s.calc.p{n}.status"))),
        );
        out.element(
            ui::button(field("remove"))
                .parent(&row)
                .label("Remove")
                .variant("plain")
                .on_press(Handler::action("remove").payload(n)),
        );
    }

    /// Recomputes everything from the signals the action carried.
    fn recompute(&self, out: &mut Outbox, action: Option<&Action>) {
        let read = |path: String| {
            action
                .and_then(|a| a.signal(&path))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        let entries: Vec<Entry> = self
            .ids
            .iter()
            .map(|n| Entry {
                share: read(format!("people.p{n}.share")),
                paid: read(format!("people.p{n}.paid")),
            })
            .collect();
        let report = split(&read("total".into()), &entries);
        let (mut calc, mut err) = (Map::new(), Map::new());
        for (n, line) in self.ids.iter().zip(&report.lines) {
            let (text, kind) = status(line.share - line.paid);
            calc.insert(
                format!("p{n}"),
                json!({"due": format!("share {}", money(line.share)), "status": text, "kind": kind}),
            );
            err.insert(
                format!("p{n}"),
                json!({"share": line.share_error, "paid": line.paid_error}),
            );
        }
        err.insert("total".into(), report.total_error.clone().into());
        let paid: i64 = report.lines.iter().map(|l| l.paid).sum();
        let (left_text, left_kind) = match report.left {
            0 if report.total > 0 => ("Everything is paid.".to_owned(), "success"),
            0 => ("Enter the total to start.".to_owned(), "info"),
            l if l > 0 => (format!("Still to be paid: {}", money(l)), "warning"),
            l => (format!("Paid {} more than the total.", money(-l)), "info"),
        };
        out.signal(json!({
            "calc": calc,
            "err": err,
            "summary": {
                "total": format!("Total {}", money(report.total)),
                "paid": format!("Paid so far {}", money(paid)),
                "left": left_text,
                "left_kind": left_kind,
                "note": report.note,
            },
        }));
    }
}

impl App for BillSplit {
    fn hello(&mut self, out: &mut Outbox, _: &BTreeMap<String, String>) {
        out.signal(json!({"total": ""}));
        out.element(ui::column("main").gap("large"));
        out.element(
            ui::text("title")
                .parent("main")
                .variant("h1")
                .text("Splitwise-ish — Split the Bill"),
        );
        out.element(
            ui::card("bill")
                .parent("main")
                .title("The bill")
                .gap("medium"),
        );
        out.element(
            ui::input("total")
                .parent("bill")
                .label("Total amount")
                .placeholder("e.g. 120.50")
                .bind("total")
                .error(cel("s.err.total"))
                .on_change(Handler::action("edit")),
        );
        out.element(
            ui::card("summary")
                .parent("main")
                .title("Where we stand")
                .gap("medium"),
        );
        out.element(
            ui::row("figures")
                .parent("summary")
                .gap("medium")
                .wrap(true),
        );
        out.element(
            ui::badge("total_badge")
                .parent("figures")
                .variant("info")
                .text(cel("s.summary.total")),
        );
        out.element(
            ui::badge("paid_badge")
                .parent("figures")
                .variant("info")
                .text(cel("s.summary.paid")),
        );
        out.element(
            ui::notice("left")
                .parent("summary")
                .variant(cel("s.summary.left_kind"))
                .text(cel("s.summary.left")),
        );
        out.element(
            ui::notice("note")
                .parent("summary")
                .variant("warning")
                .visible(cel("s.summary.note != ''"))
                .text(cel("s.summary.note")),
        );
        out.element(
            ui::card("people_card")
                .parent("main")
                .title("People")
                .gap("medium"),
        );
        out.element(
            ui::list("people")
                .parent("people_card")
                .empty("No one yet: add the first person."),
        );
        out.element(
            ui::button("add")
                .parent("people_card")
                .label("Add person")
                .variant("primary")
                .on_press(Handler::action("add")),
        );
        for _ in 0..2 {
            self.person(out);
        }
        self.recompute(out, None);
    }

    fn action(&mut self, out: &mut Outbox, action: &Action) {
        match action.name.as_str() {
            "add" => self.person(out),
            "remove" => {
                if let Some(n) = action.payload_as::<u32>().filter(|n| self.ids.contains(n)) {
                    self.ids.retain(|i| *i != n);
                    out.remove(format!("row_{n}"));
                    out.signal(json!({
                        "people": {format!("p{n}"): null},
                        "calc": {format!("p{n}"): null},
                        "err": {format!("p{n}"): null},
                    }));
                }
            }
            "edit" => {}
            _ => return,
        }
        self.recompute(out, Some(action));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hypereffect::server::Loopback;

    fn entry(share: &str, paid: &str) -> Entry {
        Entry {
            share: share.into(),
            paid: paid.into(),
        }
    }

    #[test]
    fn blank_shares_split_the_rest() {
        let r = split("100", &[entry("50%", "20"), entry("", ""), entry("", "")]);
        let shares: Vec<i64> = r.lines.iter().map(|l| l.share).collect();
        assert_eq!(shares, [5000, 2500, 2500]);
        assert_eq!(r.left, 8000);
        assert_eq!(r.note, "");
    }

    #[test]
    fn cents_are_never_lost() {
        let r = split("100", &[entry("", ""), entry("", ""), entry("", "")]);
        let shares: Vec<i64> = r.lines.iter().map(|l| l.share).collect();
        assert_eq!(shares, [3334, 3333, 3333]);
        assert_eq!(shares.iter().sum::<i64>(), 10000);
    }

    #[test]
    fn mixed_amounts_and_problems() {
        // An amount, a percent and a blank.
        let r = split("200", &[entry("30", "0"), entry("25%", ""), entry("", "")]);
        let shares: Vec<i64> = r.lines.iter().map(|l| l.share).collect();
        assert_eq!(shares, [3000, 5000, 12000]);
        // Over-assigned: blanks get nothing.
        let r = split("100", &[entry("80", ""), entry("40", ""), entry("", "")]);
        assert_eq!(r.lines[2].share, 0);
        assert_eq!(r.note, "The shares exceed the total by 20.00.");
        // Nothing left for anyone to take.
        let r = split("100", &[entry("30", ""), entry("20%", "")]);
        assert!(r.note.starts_with("50.00 of the total is not assigned"));
        // Bad input is flagged and counted as nothing.
        let r = split("abc", &[entry("x%", "-1")]);
        assert_eq!(r.total_error, "Not a valid amount.");
        assert_eq!(r.lines[0].share_error, "Not a valid percent.");
        assert_eq!(r.lines[0].paid_error, "Not a valid amount.");
    }

    #[test]
    fn people_see_what_they_owe() {
        let mut l = Loopback::<BillSplit>::new();
        assert_eq!(
            l.client.attr("left", "text"),
            Some(json!("Enter the total to start."))
        );
        assert!(l.trigger("add", "onPress"));
        l.write("total", json!("100"));
        l.write("people.p1.name", json!("Ann"));
        l.write("people.p1.share", json!("50%"));
        l.write("people.p1.paid", json!("20"));
        l.write("people.p2.paid", json!("45"));
        assert!(l.trigger("total", "onChange"));
        assert_eq!(l.client.attr("due_1", "text"), Some(json!("share 50.00")));
        assert_eq!(l.client.attr("status_1", "text"), Some(json!("owes 30.00")));
        assert_eq!(l.client.attr("status_1", "variant"), Some(json!("warning")));
        // The other two split the remaining 50 equally; one overpaid.
        assert_eq!(l.client.attr("due_2", "text"), Some(json!("share 25.00")));
        assert_eq!(
            l.client.attr("status_2", "text"),
            Some(json!("overpaid by 20.00"))
        );
        assert_eq!(l.client.attr("status_2", "variant"), Some(json!("info")));
        assert_eq!(l.client.attr("status_3", "text"), Some(json!("owes 25.00")));
        assert_eq!(
            l.client.attr("left", "text"),
            Some(json!("Still to be paid: 35.00"))
        );
        assert_eq!(
            l.client.attr("paid_badge", "text"),
            Some(json!("Paid so far 65.00"))
        );
        // Editing a share updates the others.
        l.write("people.p1.share", json!("x"));
        assert!(l.trigger("share_1", "onChange"));
        assert_eq!(
            l.client.attr("share_1", "error"),
            Some(json!("Use a percent, an amount, or leave blank."))
        );
        l.write("people.p1.share", json!("100"));
        assert!(l.trigger("share_1", "onChange"));
        assert_eq!(l.client.attr("share_1", "error"), Some(json!("")));
        assert_eq!(l.client.attr("status_1", "text"), Some(json!("owes 80.00")));
        assert_eq!(l.client.attr("status_3", "text"), Some(json!("settled")));
        // Removing a person drops their row and redistributes.
        assert!(l.trigger("remove_1", "onPress"));
        assert!(!l.client.contains("row_1"));
        assert_eq!(l.client.value("people.p1"), None);
        assert_eq!(l.client.attr("status_2", "text"), Some(json!("owes 5.00")));
        assert_eq!(
            l.client.attr("left", "text"),
            Some(json!("Still to be paid: 55.00"))
        );
        assert!(l.client.take_diagnostics().is_empty());
    }
}
