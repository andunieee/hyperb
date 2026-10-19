//! The SaaS admin panel demo, for a fictional product ("Cumulus"). All the
//! data is made up.
//!
//! An admin sees revenue figures and charts, searches and filters the
//! customers, opens one to change its plan and seats, suspend or cancel it,
//! sets the price of each plan, flips feature flags and reads the audit log.
//! The workspace is remembered by the admin's key, so changes survive
//! reconnecting, and every change is stamped with the key in the audit log.

use super::chart::{draw, Point, PALETTE};
use super::common::{group, key, menu, number, replace, text};
use super::img;
use super::store;
use hyperb_core::identity::PublicKey;
use hypereffect::{cel, json, ui, Action, App, Element, Handler, Map, Outbox, Value};
use std::sync::LazyLock;

pub(crate) static WORKSPACES: LazyLock<store::Memory<Workspace>> =
    LazyLock::new(store::Memory::new);

const PAGE: usize = 6;
const AUDIT_SHOWN: usize = 12;
const AUDIT_KEPT: usize = 60;
const PLANS: [&str; 4] = ["free", "starter", "pro", "enterprise"];
const STATUSES: [&str; 4] = ["active", "trial", "suspended", "churned"];

#[derive(Clone)]
struct Customer {
    id: u32,
    name: String,
    email: String,
    plan: String,
    status: String,
    seats: u32,
    joined: String,
}

#[derive(Clone)]
struct Flag {
    key: &'static str,
    label: &'static str,
    enabled: bool,
    rollout: u32,
}

#[derive(Clone)]
pub(crate) struct Workspace {
    customers: Vec<Customer>,
    /// Dollars per seat per month, by plan.
    prices: [u32; 4],
    flags: Vec<Flag>,
    audit: Vec<String>,
    seq: u32,
}

impl Default for Workspace {
    fn default() -> Self {
        let names = [
            "Northwind",
            "Globex",
            "Initech",
            "Umbrella",
            "Hooli",
            "Stark",
            "Wayne",
            "Wonka",
            "Soylent",
            "Cyberdyne",
            "Tyrell",
            "Vandelay",
        ];
        let suffix = ["Labs", "Systems", "Co", "Group"];
        let mut state = 99u64;
        let mut next = |below: u64| {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (state >> 33) % below
        };
        let customers = (1..=36)
            .map(|id| {
                let base = names[(id as usize * 5) % names.len()];
                let name = format!("{base} {}", suffix[next(4) as usize]);
                let status = match next(10) {
                    0 => "churned",
                    1 => "suspended",
                    2 | 3 => "trial",
                    _ => "active",
                };
                let plan = PLANS[next(4) as usize];
                Customer {
                    id,
                    email: format!("ops@{}.example", name.to_lowercase().replace(' ', "-")),
                    name,
                    plan: plan.into(),
                    status: status.into(),
                    seats: if plan == "free" {
                        1 + next(3) as u32
                    } else {
                        2 + next(60) as u32
                    },
                    joined: format!("2025-{:02}-{:02}", next(12) + 1, next(28) + 1),
                }
            })
            .collect();
        let flag = |key, label, enabled, rollout| Flag {
            key,
            label,
            enabled,
            rollout,
        };
        Workspace {
            customers,
            prices: [0, 12, 29, 79],
            flags: vec![
                flag("new_editor", "New editor", true, 100),
                flag("ai_summaries", "AI summaries", true, 25),
                flag("sso", "Single sign-on", false, 0),
                flag("usage_billing", "Usage-based billing", false, 10),
                flag("dark_mode", "Dark mode", true, 100),
            ],
            audit: Vec::new(),
            seq: 0,
        }
    }
}

impl Workspace {
    fn mrr_of(&self, c: &Customer) -> i64 {
        if c.status == "active" {
            let plan = PLANS.iter().position(|p| *p == c.plan).unwrap_or(0);
            i64::from(self.prices[plan]) * i64::from(c.seats)
        } else {
            0
        }
    }

    fn mrr(&self) -> i64 {
        self.customers.iter().map(|c| self.mrr_of(c)).sum()
    }

    fn count(&self, status: &str) -> usize {
        self.customers.iter().filter(|c| c.status == status).count()
    }

    fn log(&mut self, who: &PublicKey, what: impl AsRef<str>) {
        self.seq += 1;
        self.audit
            .push(format!("#{} {} — {}", self.seq, short(who), what.as_ref()));
        let extra = self.audit.len().saturating_sub(AUDIT_KEPT);
        self.audit.drain(..extra);
    }
}

fn short(key: &PublicKey) -> String {
    let hex = key.to_string();
    format!("{}…", &hex[..8])
}

fn dollars(n: i64) -> String {
    format!("${}", group(&n.to_string()))
}

#[derive(Default)]
pub struct Saas {
    key: Option<PublicKey>,
    ws: Workspace,
    q: String,
    plan: String,
    status: String,
    page: usize,
    rows: Vec<String>,
}

impl Saas {
    fn matching(&self) -> Vec<&Customer> {
        let q = self.q.to_lowercase();
        self.ws
            .customers
            .iter()
            .filter(|c| q.is_empty() || c.name.to_lowercase().contains(&q) || c.email.contains(&q))
            .filter(|c| self.plan == "all" || c.plan == self.plan)
            .filter(|c| self.status == "all" || c.status == self.status)
            .collect()
    }

    fn save(&mut self) {
        if let Some(k) = self.key {
            WORKSPACES.update(&k, |w| *w = self.ws.clone());
        }
    }

    fn audit(&mut self, what: impl AsRef<str>) {
        if let Some(k) = self.key {
            self.ws.log(&k, what);
        }
        self.save();
    }

    fn show_customers(&mut self, out: &mut Outbox) {
        let found: Vec<Customer> = self.matching().into_iter().cloned().collect();
        let pages = found.len().div_ceil(PAGE).max(1);
        self.page = self.page.min(pages - 1);
        out.signal(json!({"view": {
            "count": format!("{} customer{}", found.len(), if found.len() == 1 { "" } else { "s" }),
            "page": format!("Page {} of {pages}", self.page + 1),
            "has_prev": self.page > 0,
            "has_next": self.page + 1 < pages,
        }}));
        let mut els: Vec<Element> = Vec::new();
        for c in found.iter().skip(self.page * PAGE).take(PAGE) {
            let id = format!("c_{}", c.id);
            els.push(
                ui::row(&id)
                    .parent("customer_list")
                    .gap("medium")
                    .wrap(true),
            );
            els.push(
                ui::text(format!("{id}_info"))
                    .parent(&id)
                    .text(format!("{} · {}", c.name, c.email)),
            );
            els.push(
                ui::badge(format!("{id}_plan"))
                    .parent(&id)
                    .variant("info")
                    .text(c.plan.as_str()),
            );
            els.push(
                ui::badge(format!("{id}_status"))
                    .parent(&id)
                    .variant(match c.status.as_str() {
                        "active" => "success",
                        "trial" => "info",
                        "suspended" => "warning",
                        _ => "failure",
                    })
                    .text(c.status.as_str()),
            );
            els.push(
                ui::text(format!("{id}_mrr"))
                    .parent(&id)
                    .variant("caption")
                    .text(format!(
                        "{}/mo · {} seats",
                        dollars(self.ws.mrr_of(c)),
                        c.seats
                    )),
            );
            els.push(
                ui::button(format!("{id}_open"))
                    .parent(&id)
                    .label("Open")
                    .on_press(Handler::action("open").payload(c.id)),
            );
        }
        replace(out, &mut self.rows, els);
    }

    fn show_dashboard(&self, out: &mut Outbox) {
        let ws = &self.ws;
        let mrr = ws.mrr();
        let active = ws.count("active");
        let by_plan: Vec<Point> = PLANS
            .iter()
            .map(|p| Point {
                label: (*p).into(),
                value: ws
                    .customers
                    .iter()
                    .filter(|c| c.plan == *p)
                    .map(|c| ws.mrr_of(c))
                    .sum::<i64>() as f64,
            })
            .collect();
        let by_status: Vec<Point> = STATUSES
            .iter()
            .map(|s| Point {
                label: (*s).into(),
                value: ws.count(s) as f64,
            })
            .filter(|p| p.value > 0.0)
            .collect();
        let uri = |kind, title, points: &[Point]| {
            draw(kind, title, points, PALETTE[0])
                .map(|png| img::data_uri(&png))
                .unwrap_or_default()
        };
        out.signal(json!({
            "stats": {
                "mrr": format!("MRR {}", dollars(mrr)),
                "arr": format!("ARR {}", dollars(mrr * 12)),
                "active": format!("{active} active"),
                "trial": format!("{} on trial", ws.count("trial")),
                "churned": format!("{} churned", ws.count("churned")),
                "arpa": format!("ARPA {}", dollars(if active > 0 { mrr / active as i64 } else { 0 })),
            },
            "charts": {
                "mrr": uri("bar", "MRR by plan ($)", &by_plan),
                "status": uri("pie", "Customers by status", &by_status),
            },
        }));
    }

    fn show_audit(&self, out: &mut Outbox) {
        let mut lines: Vec<String> = self
            .ws
            .audit
            .iter()
            .rev()
            .take(AUDIT_SHOWN)
            .cloned()
            .collect();
        lines.resize(AUDIT_SHOWN, String::new());
        out.signal(json!({"audit": lines}));
    }

    fn show_flags(&self, out: &mut Outbox) {
        let mut flags = Map::new();
        for f in &self.ws.flags {
            flags.insert(
                f.key.into(),
                json!({"enabled": f.enabled, "rollout": f.rollout}),
            );
        }
        out.signal(json!({"flags": flags, "prices": {
            "starter": self.ws.prices[1], "pro": self.ws.prices[2], "enterprise": self.ws.prices[3],
        }}));
    }

    fn show_all(&mut self, out: &mut Outbox) {
        self.show_dashboard(out);
        self.show_customers(out);
        self.show_audit(out);
        self.show_flags(out);
    }

    fn select(&self, out: &mut Outbox, id: u32) {
        let Some(c) = self.ws.customers.iter().find(|c| c.id == id) else {
            return out.signal(json!({"sel": {"id": 0}}));
        };
        out.signal(json!({"sel": {
            "id": c.id, "name": c.name, "email": c.email, "plan": c.plan, "seats": c.seats,
            "status": c.status,
            "summary": format!("{} · customer since {} · {}/mo", c.email, c.joined, dollars(self.ws.mrr_of(c))),
        }}));
    }

    fn customer(&mut self, id: u32) -> Option<&mut Customer> {
        self.ws.customers.iter_mut().find(|c| c.id == id)
    }
}

impl App for Saas {
    fn hello(&mut self, out: &mut Outbox, signals: &Map<String, Value>) {
        self.key = key(signals);
        self.ws = self
            .key
            .map(|k| WORKSPACES.snapshot(&k))
            .unwrap_or_default();
        (self.q, self.plan, self.status) = (String::new(), "all".into(), "all".into());
        out.signal(json!({
            "who": self.key.map_or("unknown".into(), |k| short(&k)),
            "filters": {"q": "", "plan": "all", "status": "all"},
            "menu": {
                "plans": menu(&[("all", "All plans"), ("free", "Free"), ("starter", "Starter"), ("pro", "Pro"), ("enterprise", "Enterprise")]),
                "plan_choice": menu(&PLANS.map(|p| (p, p))),
                "statuses": menu(&[("all", "Any status"), ("active", "Active"), ("trial", "Trial"), ("suspended", "Suspended"), ("churned", "Churned")]),
            },
            "sel": {"id": 0, "name": "", "email": "", "plan": "free", "seats": 1, "status": "", "summary": ""},
            "feedback": {"kind": "info", "text": ""},
        }));
        out.element(ui::column("main").gap("large"));
        out.element(
            ui::text("title")
                .parent("main")
                .variant("h1")
                .text("Cumulus — Admin"),
        );
        out.element(
            ui::text("who_line")
                .parent("main")
                .variant("caption")
                .text(cel(
                    "'Signed in as admin ' + s.who + ' — every change is recorded under this key.'",
                )),
        );
        out.element(
            ui::notice("feedback")
                .parent("main")
                .variant(cel("s.feedback.kind"))
                .visible(cel("s.feedback.text != ''"))
                .text(cel("s.feedback.text")),
        );
        out.element(ui::tabs("tabs").parent("main"));

        // Dashboard.
        out.element(ui::tab("dash").parent("tabs").label("Dashboard"));
        out.element(ui::row("kpis").parent("dash").gap("small").wrap(true));
        for (id, variant) in [
            ("mrr", "success"),
            ("arr", "success"),
            ("active", "info"),
            ("trial", "info"),
            ("churned", "failure"),
            ("arpa", "neutral"),
        ] {
            out.element(
                ui::badge(format!("kpi_{id}"))
                    .parent("kpis")
                    .variant(variant)
                    .text(cel(format!("s.stats.{id}"))),
            );
        }
        out.element(
            ui::image("chart_mrr")
                .parent("dash")
                .alt("MRR by plan")
                .src(cel("s.charts.mrr")),
        );
        out.element(
            ui::image("chart_status")
                .parent("dash")
                .alt("Customers by status")
                .src(cel("s.charts.status")),
        );

        // Customers.
        out.element(ui::tab("customers").parent("tabs").label("Customers"));
        out.element(
            ui::input("search")
                .parent("customers")
                .label("Search")
                .bind("filters.q")
                .placeholder("Name or email")
                .on_change(Handler::action("filter")),
        );
        out.element(
            ui::row("filter_row")
                .parent("customers")
                .gap("medium")
                .wrap(true),
        );
        out.element(
            ui::select("plan_filter")
                .parent("filter_row")
                .label("Plan")
                .bind("filters.plan")
                .options(cel("s.menu.plans"))
                .on_change(Handler::action("filter")),
        );
        out.element(
            ui::select("status_filter")
                .parent("filter_row")
                .label("Status")
                .bind("filters.status")
                .options(cel("s.menu.statuses"))
                .on_change(Handler::action("filter")),
        );
        out.element(
            ui::badge("count")
                .parent("customers")
                .text(cel("s.view.count")),
        );
        out.element(
            ui::list("customer_list")
                .parent("customers")
                .empty("No customer matches."),
        );
        out.element(ui::row("pager").parent("customers").gap("small"));
        out.element(
            ui::button("prev")
                .parent("pager")
                .label("← Previous")
                .disabled(cel("!s.view.has_prev"))
                .on_press(Handler::action("page").payload(-1)),
        );
        out.element(ui::badge("page").parent("pager").text(cel("s.view.page")));
        out.element(
            ui::button("next")
                .parent("pager")
                .label("Next →")
                .disabled(cel("!s.view.has_next"))
                .on_press(Handler::action("page").payload(1)),
        );

        // Billing.
        out.element(ui::tab("billing").parent("tabs").label("Pricing"));
        out.element(
            ui::text("price_help")
                .parent("billing")
                .variant("caption")
                .text("Dollars per seat per month. The free plan is always $0."),
        );
        for plan in ["starter", "pro", "enterprise"] {
            out.element(
                ui::slider(format!("price_{plan}"))
                    .parent("billing")
                    .label(plan)
                    .min(1)
                    .max(200)
                    .bind(format!("prices.{plan}")),
            );
        }
        out.element(
            ui::button("apply_prices")
                .parent("billing")
                .label("Apply prices")
                .variant("primary")
                .on_press(Handler::action("prices")),
        );

        // Flags.
        out.element(ui::tab("flags_tab").parent("tabs").label("Feature flags"));
        for f in &self.ws.flags {
            let k = f.key;
            out.element(
                ui::card(format!("flag_{k}"))
                    .parent("flags_tab")
                    .gap("small"),
            );
            out.element(
                ui::checkbox(format!("flag_{k}_on"))
                    .parent(format!("flag_{k}"))
                    .label(f.label)
                    .variant("switch")
                    .bind(format!("flags.{k}.enabled")),
            );
            out.element(
                ui::slider(format!("flag_{k}_pct"))
                    .parent(format!("flag_{k}"))
                    .label("Rollout %")
                    .min(0)
                    .max(100)
                    .step(5)
                    .bind(format!("flags.{k}.rollout"))
                    .disabled(cel(format!("!s.flags.{k}.enabled"))),
            );
            out.element(
                ui::button(format!("flag_{k}_save"))
                    .parent(format!("flag_{k}"))
                    .label("Save")
                    .on_press(Handler::action("flag").payload(k)),
            );
        }

        // Audit.
        out.element(ui::tab("audit_tab").parent("tabs").label("Audit log"));
        for k in 0..AUDIT_SHOWN {
            out.element(
                ui::text(format!("audit_{k}"))
                    .parent("audit_tab")
                    .variant("code")
                    .visible(cel(format!("s.audit[{k}] != ''")))
                    .text(cel(format!("s.audit[{k}]"))),
            );
        }
        out.element(
            ui::button("reset")
                .parent("audit_tab")
                .label("Reset demo data")
                .variant("failure")
                .on_press(Handler::action("reset")),
        );

        // The customer being edited.
        out.element(
            ui::dialog("detail")
                .open(cel("s.sel.id > 0"))
                .title(cel("s.sel.name"))
                .on_dismiss(Handler::action("close")),
        );
        out.element(
            ui::text("sel_summary")
                .parent("detail")
                .text(cel("s.sel.summary")),
        );
        out.element(ui::badge("sel_status").parent("detail").variant(cel("s.sel.status == 'active' ? 'success' : (s.sel.status == 'churned' ? 'failure' : 'warning')")).text(cel("s.sel.status")));
        out.element(
            ui::select("sel_plan")
                .parent("detail")
                .label("Plan")
                .bind("sel.plan")
                .options(cel("s.menu.plan_choice"))
                .disabled(cel("s.sel.status == 'churned'")),
        );
        out.element(
            ui::slider("sel_seats")
                .parent("detail")
                .label("Seats")
                .min(1)
                .max(500)
                .bind("sel.seats")
                .disabled(cel("s.sel.status == 'churned'")),
        );
        out.element(
            ui::row("sel_actions")
                .parent("detail")
                .gap("small")
                .wrap(true),
        );
        out.element(
            ui::button("sel_save")
                .parent("sel_actions")
                .label("Save")
                .variant("primary")
                .disabled(cel("s.sel.status == 'churned'"))
                .on_press(Handler::action("save")),
        );
        out.element(
            ui::button("sel_suspend")
                .parent("sel_actions")
                .label(cel(
                    "s.sel.status == 'suspended' ? 'Reactivate' : 'Suspend'",
                ))
                .disabled(cel("s.sel.status == 'churned'"))
                .on_press(Handler::action("suspend")),
        );
        out.element(
            ui::button("sel_impersonate")
                .parent("sel_actions")
                .label("Impersonate")
                .disabled(cel("s.sel.status == 'churned'"))
                .on_press(Handler::action("impersonate")),
        );
        out.element(
            ui::button("sel_cancel")
                .parent("sel_actions")
                .label("Cancel subscription")
                .variant("failure")
                .disabled(cel("s.sel.status == 'churned'"))
                .on_press(Handler::action("cancel")),
        );
        out.element(
            ui::button("sel_close")
                .parent("sel_actions")
                .label("Close")
                .variant("plain")
                .on_press(Handler::action("close")),
        );
        self.show_all(out);
    }

    fn action(&mut self, out: &mut Outbox, action: &Action) {
        let say = |out: &mut Outbox, kind: &str, t: &str| {
            out.signal(json!({"feedback": {"kind": kind, "text": t}}))
        };
        say(out, "info", "");
        let id: u32 = action.payload_as().unwrap_or(0);
        let sel_id = action.signal("sel.id").and_then(Value::as_u64).unwrap_or(0) as u32;
        match action.name.as_str() {
            "filter" => {
                self.q = text(action, "filters.q");
                self.plan = text(action, "filters.plan");
                self.status = text(action, "filters.status");
                self.page = 0;
                self.show_customers(out);
            }
            "page" => {
                let step: i64 = action.payload_as().unwrap_or(0);
                self.page = (self.page as i64 + step).max(0) as usize;
                self.show_customers(out);
            }
            "open" => self.select(out, id),
            "close" => out.signal(json!({"sel": {"id": 0}})),
            "save" => {
                let plan = text(action, "sel.plan");
                let seats = number(action, "sel.seats").unwrap_or(1.0);
                if !PLANS.contains(&plan.as_str()) || !(1.0..=500.0).contains(&seats) {
                    return say(out, "failure", "Pick a plan and between 1 and 500 seats.");
                }
                let Some(c) = self.customer(sel_id) else {
                    return;
                };
                let note = format!("set {} to {plan}, {} seats", c.name, seats as u32);
                (c.plan, c.seats) = (plan, seats as u32);
                self.audit(note);
                say(out, "success", "Saved.");
                self.show_all(out);
                self.select(out, sel_id);
            }
            "suspend" => {
                let Some(c) = self.customer(sel_id) else {
                    return;
                };
                let (now, verb) = if c.status == "suspended" {
                    ("active", "reactivated")
                } else {
                    ("suspended", "suspended")
                };
                c.status = now.into();
                let note = format!("{verb} {}", c.name);
                self.audit(note);
                self.show_all(out);
                self.select(out, sel_id);
            }
            "cancel" => {
                let Some(c) = self.customer(sel_id) else {
                    return;
                };
                c.status = "churned".into();
                let note = format!("cancelled the subscription of {}", c.name);
                self.audit(note);
                say(out, "warning", "Subscription cancelled.");
                self.show_all(out);
                self.select(out, sel_id);
            }
            "impersonate" => {
                let Some(c) = self.customer(sel_id) else {
                    return;
                };
                let note = format!("started an impersonation session as {}", c.name);
                self.audit(note);
                say(
                    out,
                    "info",
                    "Impersonation link created: it expires in 15 minutes.",
                );
                self.show_audit(out);
            }
            "prices" => {
                let mut prices = self.ws.prices;
                for (i, plan) in PLANS.iter().enumerate().skip(1) {
                    prices[i] = number(action, &format!("prices.{plan}"))
                        .unwrap_or(f64::from(prices[i]))
                        .clamp(1.0, 200.0) as u32;
                }
                if prices[1] >= prices[2] || prices[2] >= prices[3] {
                    return say(
                        out,
                        "failure",
                        "Each plan must cost more than the one below it.",
                    );
                }
                let before = self.ws.mrr();
                self.ws.prices = prices;
                let after = self.ws.mrr();
                self.audit(format!(
                    "changed prices to ${}/${}/${}: MRR {} → {}",
                    prices[1],
                    prices[2],
                    prices[3],
                    dollars(before),
                    dollars(after)
                ));
                say(
                    out,
                    "success",
                    &format!("Prices applied. MRR is now {}.", dollars(after)),
                );
                self.show_all(out);
            }
            "flag" => {
                let key_name: String = action.payload_as().unwrap_or_default();
                let enabled = action
                    .signal(&format!("flags.{key_name}.enabled"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let rollout = number(action, &format!("flags.{key_name}.rollout"))
                    .unwrap_or(0.0)
                    .clamp(0.0, 100.0) as u32;
                if let Some(f) = self.ws.flags.iter_mut().find(|f| f.key == key_name) {
                    (f.enabled, f.rollout) = (enabled, rollout);
                    let note = format!(
                        "{} {} at {rollout}%",
                        if enabled { "enabled" } else { "disabled" },
                        f.label
                    );
                    self.audit(note);
                    say(out, "success", "Flag saved.");
                    self.show_audit(out);
                }
            }
            "reset" => {
                self.ws = Workspace::default();
                self.audit("reset the demo data");
                say(out, "info", "Demo data restored.");
                self.show_all(out);
                self.show_audit(out);
                out.signal(json!({"sel": {"id": 0}}));
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{connect, trigger};
    use hyperb_core::identity::Identity;

    fn stat(l: &hypereffect::server::Loopback<Saas>, id: &str) -> String {
        l.client
            .attr(id, "text")
            .unwrap()
            .as_str()
            .unwrap()
            .to_owned()
    }

    #[test]
    fn dashboard_and_customers() {
        let key = Identity::generate().public();
        let mut l = connect::<Saas>(key);
        assert!(stat(&l, "kpi_mrr").starts_with("MRR $"));
        assert!(l
            .client
            .attr("chart_mrr", "src")
            .unwrap()
            .as_str()
            .unwrap()
            .starts_with("data:image/png"));
        assert_eq!(stat(&l, "count"), "36 customers");
        assert_eq!(stat(&l, "page"), "Page 1 of 6");
        assert_eq!(l.client.attr("prev", "disabled"), Some(json!(true)));
        assert!(l.client.contains("c_1_open") || l.client.contains("c_2_open"));
        // Paging replaces the rows.
        let first = l.client.children(Some("customer_list")).to_vec();
        assert_eq!(first.len(), PAGE);
        assert!(trigger(&mut l, key, "next", "onPress"));
        assert_ne!(l.client.children(Some("customer_list")), first.as_slice());
        // Filtering narrows the list and goes back to the first page.
        l.write("filters.plan", json!("enterprise"));
        assert!(trigger(&mut l, key, "plan_filter", "onChange"));
        let n: usize = stat(&l, "count")
            .split(' ')
            .next()
            .unwrap()
            .parse()
            .unwrap();
        assert!(n > 0 && n < 36);
        assert_eq!(stat(&l, "page").split(' ').nth(1), Some("1"));
        l.write("filters.q", json!("no such customer"));
        assert!(trigger(&mut l, key, "search", "onChange"));
        assert_eq!(stat(&l, "count"), "0 customers");
        assert!(l.client.children(Some("customer_list")).is_empty());
        let d = l.client.take_diagnostics();
        assert!(d.is_empty(), "{d:?}");
    }

    #[test]
    fn editing_a_customer_is_audited_and_remembered() {
        let key = Identity::generate().public();
        let mut l = connect::<Saas>(key);
        let mrr_before = stat(&l, "kpi_mrr");
        // Find an active customer on the first page and open it.
        let id = l
            .client
            .children(Some("customer_list"))
            .iter()
            .find(|r| l.client.attr(&format!("{r}_status"), "text") == Some(json!("active")))
            .map(|r| r.trim_start_matches("c_").parse::<u32>().unwrap())
            .expect("an active customer");
        assert_eq!(l.client.attr("detail", "open"), Some(json!(false)));
        assert!(trigger(&mut l, key, &format!("c_{id}_open"), "onPress"));
        assert_eq!(l.client.attr("detail", "open"), Some(json!(true)));
        l.write("sel.plan", json!("enterprise"));
        l.write("sel.seats", json!(100));
        assert!(trigger(&mut l, key, "sel_save", "onPress"));
        assert_eq!(l.client.attr("feedback", "text"), Some(json!("Saved.")));
        if l.client.contains(&format!("c_{id}_mrr")) {
            let row = l.client.attr(&format!("c_{id}_mrr"), "text").unwrap();
            assert!(row.as_str().unwrap().ends_with("100 seats"));
        }
        assert_ne!(stat(&l, "kpi_mrr"), mrr_before);
        assert!(l
            .client
            .attr("audit_0", "text")
            .unwrap()
            .as_str()
            .unwrap()
            .contains("to enterprise, 100 seats"));
        // Suspending drops its revenue; reactivating brings it back.
        let with = stat(&l, "kpi_mrr");
        assert!(trigger(&mut l, key, "sel_suspend", "onPress"));
        assert_eq!(
            l.client.attr("sel_suspend", "label"),
            Some(json!("Reactivate"))
        );
        assert_ne!(stat(&l, "kpi_mrr"), with);
        assert!(trigger(&mut l, key, "sel_suspend", "onPress"));
        assert_eq!(stat(&l, "kpi_mrr"), with);
        // Impersonating is recorded too.
        assert!(trigger(&mut l, key, "sel_impersonate", "onPress"));
        assert!(l
            .client
            .attr("audit_0", "text")
            .unwrap()
            .as_str()
            .unwrap()
            .contains("impersonation"));
        // Cancelling locks the customer.
        assert!(trigger(&mut l, key, "sel_cancel", "onPress"));
        assert_eq!(l.client.attr("sel_save", "disabled"), Some(json!(true)));
        assert!(!trigger(&mut l, key, "sel_save", "onPress"));
        assert!(trigger(&mut l, key, "sel_close", "onPress"));
        assert_eq!(l.client.attr("detail", "open"), Some(json!(false)));
        // A new session with the same key finds the changes and the log.
        drop(l);
        let l = connect::<Saas>(key);
        assert!(l
            .client
            .attr("audit_0", "text")
            .unwrap()
            .as_str()
            .unwrap()
            .contains("cancelled the subscription"));
        assert!(l
            .client
            .attr("audit_0", "text")
            .unwrap()
            .as_str()
            .unwrap()
            .contains(&short(&key)));
        // Another admin has a workspace of their own.
        let other = connect::<Saas>(Identity::generate().public());
        assert_eq!(other.client.attr("audit_0", "visible"), Some(json!(false)));
    }

    #[test]
    fn prices_and_flags() {
        let key = Identity::generate().public();
        let mut l = connect::<Saas>(key);
        let before = stat(&l, "kpi_mrr");
        l.write("prices.pro", json!(10));
        assert!(trigger(&mut l, key, "apply_prices", "onPress"));
        assert_eq!(l.client.attr("feedback", "variant"), Some(json!("failure")));
        l.write("prices.pro", json!(50));
        assert!(trigger(&mut l, key, "apply_prices", "onPress"));
        assert_eq!(l.client.attr("feedback", "variant"), Some(json!("success")));
        assert_ne!(stat(&l, "kpi_mrr"), before);
        // A flag's rollout slider follows its switch.
        assert_eq!(l.client.attr("flag_sso_pct", "disabled"), Some(json!(true)));
        l.write("flags.sso.enabled", json!(true));
        assert_eq!(
            l.client.attr("flag_sso_pct", "disabled"),
            Some(json!(false))
        );
        l.write("flags.sso.rollout", json!(40));
        assert!(trigger(&mut l, key, "flag_sso_save", "onPress"));
        assert!(l
            .client
            .attr("audit_0", "text")
            .unwrap()
            .as_str()
            .unwrap()
            .contains("enabled Single sign-on at 40%"));
        // Reset puts the data back.
        assert!(trigger(&mut l, key, "reset", "onPress"));
        assert_eq!(stat(&l, "kpi_mrr"), before);
        let d = l.client.take_diagnostics();
        assert!(d.is_empty(), "{d:?}");
    }
}
