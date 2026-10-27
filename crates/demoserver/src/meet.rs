//! The meeting scheduler demo, in the spirit of Doodle, built on identities.
//!
//! Someone creates a meeting: a name and some candidate times. They get a
//! short code to hand around. Others type the code, see the meeting and
//! answer with their name and the times that suit them. Everyone is
//! recognized by the public key of their session, not by a login: a person
//! has one answer per meeting, which they find again, filled in, when they come
//! back with the same key, and only the key that created a meeting can close
//! or delete it.
//!
//! Meetings live in memory, shared by all sessions. A session watches the
//! meeting it has open on its tick, so answers from other people show up
//! within a second or two.

use super::common::{feedback, key, param_key, text};
use super::store;
use hyperb_core::identity::PublicKey;
use hypereffect::{cel, json, ui, Action, App, Element, Handler, Map, Outbox, Value};
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

/// Meetings by code.
static MEETINGS: LazyLock<Mutex<HashMap<String, Meeting>>> = LazyLock::new(Default::default);
/// What is remembered about each key: the name last used, the meetings seen.
pub(crate) static PEOPLE: LazyLock<store::Memory<Person>> = LazyLock::new(store::Memory::new);

/// How long a meeting lives after its last change.
const MEETING_TTL: Duration = Duration::from_secs(24 * 60 * 60);
/// How often an open meeting is checked for news.
const WATCH_MS: u32 = 1500;
const MAX_OPTIONS: usize = 12;
const MAX_ANSWERS: usize = 50;
const MAX_OPTION_LEN: usize = 40;
const MAX_NAME_LEN: usize = 40;
const MAX_NOTE_LEN: usize = 120;
const MAX_REMEMBERED: usize = 10;
const CODE_ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";

/// Drops forgotten people and old meetings.
pub(crate) fn prune() {
    PEOPLE.prune();
    MEETINGS
        .lock()
        .unwrap()
        .retain(|_, m| m.touched.elapsed() < MEETING_TTL);
}

#[derive(Clone, Default)]
pub(crate) struct Person {
    name: String,
    /// Codes of meetings they created or answered, newest first.
    codes: Vec<String>,
}

#[derive(Clone)]
struct Meeting {
    name: String,
    description: String,
    owner: PublicKey,
    owner_name: String,
    options: Vec<String>,
    answers: Vec<(PublicKey, Answer)>,
    closed: bool,
    /// Bumped on every change, so that watchers know to refresh.
    version: u64,
    touched: Instant,
}

#[derive(Clone)]
struct Answer {
    name: String,
    picks: Vec<bool>,
    other: String,
}

impl Meeting {
    fn changed(&mut self) {
        self.version += 1;
        self.touched = Instant::now();
    }
}

fn new_code(taken: &HashMap<String, Meeting>) -> String {
    loop {
        let mut bytes = [0u8; 6];
        getrandom::getrandom(&mut bytes).expect("system random number generator");
        let code: String = bytes
            .iter()
            .map(|b| CODE_ALPHABET[*b as usize % CODE_ALPHABET.len()] as char)
            .collect();
        if !taken.contains_key(&code) {
            return code;
        }
    }
}

fn short(key: &PublicKey) -> String {
    let hex = key.to_string();
    format!("{}…{}", &hex[..8], &hex[hex.len() - 4..])
}

fn limit(s: String, max: usize) -> String {
    s.chars().take(max).collect()
}

/// Remembers that `key` took part in `code`.
fn remember(key: &PublicKey, code: &str, name: Option<&str>) {
    PEOPLE.update(key, |p| {
        if let Some(n) = name.filter(|n| !n.is_empty()) {
            p.name = n.to_owned();
        }
        p.codes.retain(|c| c != code);
        p.codes.insert(0, code.to_owned());
        p.codes.truncate(MAX_REMEMBERED);
    });
}

#[derive(Default)]
pub struct Meet {
    /// The key of this session.
    viewer: Option<PublicKey>,
    /// The meeting open in this session.
    code: Option<String>,
    /// The version of it that is on screen.
    seen: u64,
    /// How many options the open meeting has.
    options: usize,
    /// Ids of the elements that belong to the open meeting or to the list of
    /// meetings, which are replaced as a whole.
    meeting_ids: Vec<String>,
    answer_ids: Vec<String>,
    mine_ids: Vec<String>,
}

impl Meet {
    /// Replaces the elements `ids` with `new`.
    fn replace(out: &mut Outbox, ids: &mut Vec<String>, new: Vec<Element>) {
        for id in ids.drain(..) {
            out.remove(id);
        }
        for e in new {
            ids.push(e.id.clone());
            out.element(e);
        }
    }

    /// The meetings this key knows about, as buttons.
    fn show_mine(&mut self, out: &mut Outbox, key: PublicKey) {
        let meetings = MEETINGS.lock().unwrap();
        let codes: Vec<String> = PEOPLE
            .snapshot(&key)
            .codes
            .into_iter()
            .filter(|c| meetings.contains_key(c))
            .collect();
        let buttons = codes
            .iter()
            .enumerate()
            .map(|(k, code)| {
                let m = &meetings[code];
                ui::button(format!("mine_{k}"))
                    .parent("mine_list")
                    .label(format!("{} ({code})", m.name))
                    .on_press(Handler::action("open").payload(code.as_str()))
            })
            .collect();
        drop(meetings);
        out.signal(json!({"has_mine": !codes.is_empty()}));
        Self::replace(out, &mut self.mine_ids, buttons);
    }

    /// Opens a meeting in this session.
    fn enter(&mut self, out: &mut Outbox, key: PublicKey, code: &str) {
        self.close_view(out);
        let Some(m) = MEETINGS.lock().unwrap().get(code).cloned() else {
            out.signal(feedback(
                "failure",
                format!("There is no meeting with the code {code}."),
            ));
            return;
        };
        self.code = Some(code.to_owned());
        self.options = m.options.len();
        let mine = m.answers.iter().find(|(k, _)| *k == key).map(|(_, a)| a);
        let mut answer = Map::new();
        for k in 0..MAX_OPTIONS {
            let on = mine.is_some_and(|a| a.picks.get(k).copied().unwrap_or(false));
            answer.insert(format!("o{k}"), on.into());
        }
        answer.insert(
            "name".into(),
            mine.map_or_else(|| PEOPLE.snapshot(&key).name, |a| a.name.clone())
                .into(),
        );
        answer.insert("other".into(), mine.map_or("", |a| &a.other).into());
        out.signal(
            json!({"view": "meeting", "answer": answer, "feedback": {"kind": "info", "text": ""}}),
        );
        let mut elements = Vec::new();
        for (k, label) in m.options.iter().enumerate() {
            elements.push(
                ui::checkbox(format!("opt_{k}"))
                    .parent("answer_card")
                    .before("other")
                    .label(label.as_str())
                    .bind(format!("answer.o{k}"))
                    .disabled(cel("s.meeting.closed")),
            );
            elements.push(
                ui::text(format!("res_{k}"))
                    .parent("results_card")
                    .before("people")
                    .text(cel(format!("s.results.o{k}"))),
            );
        }
        Self::replace(out, &mut self.meeting_ids, elements);
        remember(&key, code, None);
        self.show_mine(out, key);
        self.refresh(out, key);
    }

    /// Back to the start page.
    fn close_view(&mut self, out: &mut Outbox) {
        self.code = None;
        Self::replace(out, &mut self.meeting_ids, Vec::new());
        Self::replace(out, &mut self.answer_ids, Vec::new());
    }

    fn go_home(&mut self, out: &mut Outbox, key: PublicKey) {
        self.close_view(out);
        out.signal(json!({"view": "home"}));
        self.show_mine(out, key);
    }

    /// Shows the open meeting as it is now.
    fn refresh(&mut self, out: &mut Outbox, key: PublicKey) {
        let Some(code) = self.code.clone() else {
            return;
        };
        let Some(m) = MEETINGS.lock().unwrap().get(&code).cloned() else {
            self.go_home(out, key);
            out.signal(feedback(
                "warning",
                format!("The meeting {code} no longer exists."),
            ));
            return;
        };
        self.seen = m.version;
        let who = |k: &PublicKey, a: &Answer| {
            let tag = if *k == key {
                " (you)"
            } else if *k == m.owner {
                " (organizer)"
            } else {
                ""
            };
            format!("{}{tag}", a.name)
        };
        let mut results = Map::new();
        let mut counts = Vec::new();
        for (k, option) in m.options.iter().enumerate() {
            let yes: Vec<String> = m
                .answers
                .iter()
                .filter(|(_, a)| a.picks.get(k).copied().unwrap_or(false))
                .map(|(pk, a)| who(pk, a))
                .collect();
            counts.push(yes.len());
            let line = if yes.is_empty() {
                format!("{option}: nobody yet")
            } else {
                format!("{option}: {} available — {}", yes.len(), yes.join(", "))
            };
            results.insert(format!("o{k}"), line.into());
        }
        let top = counts.iter().copied().max().unwrap_or(0);
        let best = if top == 0 {
            String::new()
        } else {
            let winners: Vec<&str> = m
                .options
                .iter()
                .zip(&counts)
                .filter(|(_, c)| **c == top)
                .map(|(o, _)| o.as_str())
                .collect();
            format!(
                "Best so far: {} ({top} of {} answered)",
                winners.join(" / "),
                m.answers.len()
            )
        };
        results.insert("best".into(), best.into());
        out.signal(json!({
            "meeting": {
                "name": m.name,
                "description": m.description,
                "code": code,
                "owner": format!("Created by {}", m.owner_name),
                "is_owner": m.owner == key,
                "closed": m.closed,
                "count": format!("{} answered", m.answers.len()),
            },
            "results": results,
        }));
        let rows = m
            .answers
            .iter()
            .enumerate()
            .map(|(n, (pk, a))| {
                let picked: Vec<&str> = m
                    .options
                    .iter()
                    .zip(&a.picks)
                    .filter(|(_, on)| **on)
                    .map(|(o, _)| o.as_str())
                    .collect();
                let mut line = format!(
                    "{} — {}",
                    who(pk, a),
                    if picked.is_empty() {
                        "none of the options".to_owned()
                    } else {
                        picked.join(", ")
                    }
                );
                if !a.other.is_empty() {
                    line.push_str(&format!(" · also: {}", a.other));
                }
                ui::text(format!("ans_{n}")).parent("people").text(line)
            })
            .collect();
        Self::replace(out, &mut self.answer_ids, rows);
    }

    fn create(&mut self, out: &mut Outbox, key: PublicKey, action: &Action) {
        let fail = |out: &mut Outbox, why: &str| out.signal(feedback("failure", why));
        let title = limit(text(action, "form.title"), MAX_NAME_LEN * 2);
        let name = limit(text(action, "form.name"), MAX_NAME_LEN);
        let mut options: Vec<String> = Vec::new();
        for line in text(action, "form.options").lines() {
            let line = limit(line.trim().to_owned(), MAX_OPTION_LEN);
            if !line.is_empty() && !options.contains(&line) {
                options.push(line);
            }
        }
        if title.is_empty() {
            return fail(out, "Give the meeting a name.");
        }
        if name.is_empty() {
            return fail(out, "Enter your name.");
        }
        if options.is_empty() {
            return fail(out, "Propose at least one time, one per line.");
        }
        if options.len() > MAX_OPTIONS {
            return fail(out, &format!("At most {MAX_OPTIONS} times, please."));
        }
        let code = {
            let mut meetings = MEETINGS.lock().unwrap();
            let code = new_code(&meetings);
            meetings.insert(
                code.clone(),
                Meeting {
                    name: title,
                    description: limit(text(action, "form.description"), MAX_NOTE_LEN * 2),
                    owner: key,
                    owner_name: name.clone(),
                    options,
                    answers: Vec::new(),
                    closed: false,
                    version: 1,
                    touched: Instant::now(),
                },
            );
            code
        };
        remember(&key, &code, Some(&name));
        println!("{key} created the meeting {code}");
        out.signal(json!({"form": {"title": "", "description": "", "options": ""}}));
        self.enter(out, key, &code);
        out.signal(feedback(
            "success",
            format!("Created. Share the code {code}."),
        ));
    }

    fn answer(&mut self, out: &mut Outbox, key: PublicKey, action: &Action) {
        let Some(code) = self.code.clone() else {
            return;
        };
        let name = limit(text(action, "answer.name"), MAX_NAME_LEN);
        if name.is_empty() {
            return out.signal(feedback("failure", "Enter your name to answer."));
        }
        let note = limit(text(action, "answer.other"), MAX_NOTE_LEN);
        let result = {
            let mut meetings = MEETINGS.lock().unwrap();
            match meetings.get_mut(&code) {
                None => Err("That meeting no longer exists."),
                Some(m) if m.closed => Err("The meeting is closed to new answers."),
                Some(m) => {
                    let picks = (0..m.options.len())
                        .map(|k| {
                            action
                                .signal(&format!("answer.o{k}"))
                                .and_then(Value::as_bool)
                                == Some(true)
                        })
                        .collect();
                    let answer = Answer {
                        name: name.clone(),
                        picks,
                        other: note,
                    };
                    match m.answers.iter().position(|(k, _)| *k == key) {
                        Some(i) => m.answers[i].1 = answer,
                        None if m.answers.len() >= MAX_ANSWERS => {
                            return out.signal(feedback("failure", "This meeting is full."));
                        }
                        None => m.answers.push((key, answer)),
                    }
                    m.changed();
                    Ok(())
                }
            }
        };
        match result {
            Ok(()) => {
                remember(&key, &code, Some(&name));
                out.signal(feedback(
                    "success",
                    "Your answer is saved. You can change it any time.",
                ));
                self.refresh(out, key);
            }
            Err(why) => out.signal(feedback("failure", why)),
        }
    }

    /// Runs `f` on the open meeting if `key` owns it.
    fn as_owner(
        &mut self,
        out: &mut Outbox,
        key: PublicKey,
        f: impl FnOnce(&mut HashMap<String, Meeting>, &str),
    ) -> bool {
        let Some(code) = self.code.clone() else {
            return false;
        };
        let mut meetings = MEETINGS.lock().unwrap();
        if meetings.get(&code).is_none_or(|m| m.owner != key) {
            drop(meetings);
            out.signal(feedback("failure", "Only the organizer can do that."));
            return false;
        }
        f(&mut meetings, &code);
        true
    }
}

impl App for Meet {
    fn hello(&mut self, out: &mut Outbox, params: &BTreeMap<String, String>) {
        let key = param_key(params);
        self.viewer = key;
        let name = key.map(|k| PEOPLE.snapshot(&k).name).unwrap_or_default();
        out.signal(json!({
            "view": "home",
            "me": {"id": key.map_or("unknown".to_owned(), |k| short(&k))},
            "has_mine": false,
            "form": {"title": "", "description": "", "options": "", "name": name, "code": ""},
            "answer": {"name": "", "other": ""},
            "meeting": {"name": "", "description": "", "code": "", "owner": "", "is_owner": false, "closed": false, "count": ""},
            "results": {"best": ""},
            "feedback": {"kind": "info", "text": ""},
        }));
        out.element(ui::column("main").gap("large"));
        out.element(
            ui::text("title")
                .parent("main")
                .variant("h1")
                .text("Quorum — Find a Time"),
        );
        out.element(
            ui::text("identity")
                .parent("main")
                .variant("caption")
                .text(cel("'Your identity: ' + s.me.id + ' — your answers are tied to it, so you can return and edit them.'")),
        );
        out.element(
            ui::notice("feedback")
                .parent("main")
                .variant(cel("s.feedback.kind"))
                .visible(cel("s.feedback.text != ''"))
                .text(cel("s.feedback.text")),
        );

        // The start page.
        out.element(
            ui::column("home")
                .parent("main")
                .visible(cel("s.view == 'home'")),
        );
        out.element(
            ui::card("create")
                .parent("home")
                .title("Create a meeting")
                .gap("medium"),
        );
        out.element(
            ui::input("title_in")
                .parent("create")
                .label("Meeting name")
                .bind("form.title")
                .placeholder("Team lunch"),
        );
        out.element(
            ui::input("desc_in")
                .parent("create")
                .label("Details (optional)")
                .bind("form.description")
                .placeholder("Where, what for…"),
        );
        out.element(
            ui::input("options_in")
                .parent("create")
                .label("Times to choose from, one per line")
                .bind("form.options")
                .variant("multiline")
                .placeholder("Mon 12:00\nTue 12:30\nThu 13:00"),
        );
        out.element(
            ui::input("name_in")
                .parent("create")
                .label("Your name")
                .bind("form.name"),
        );
        out.element(
            ui::button("create_btn")
                .parent("create")
                .label("Create and get a code")
                .variant("primary")
                .on_press(Handler::action("create")),
        );
        out.element(
            ui::card("join")
                .parent("home")
                .title("Join with a code")
                .gap("medium"),
        );
        out.element(
            ui::input("code_in")
                .parent("join")
                .label("Meeting code")
                .bind("form.code")
                .placeholder("e.g. K7QD4M")
                .on_submit(Handler::action("join")),
        );
        out.element(
            ui::button("join_btn")
                .parent("join")
                .label("Open meeting")
                .variant("primary")
                .on_press(Handler::action("join")),
        );
        out.element(
            ui::card("mine")
                .parent("home")
                .title("Your meetings")
                .visible(cel("s.has_mine")),
        );
        out.element(ui::list("mine_list").parent("mine"));

        // An open meeting.
        out.element(
            ui::column("meeting")
                .parent("main")
                .visible(cel("s.view == 'meeting'")),
        );
        out.element(ui::row("bar").parent("meeting").gap("medium").wrap(true));
        out.element(
            ui::button("back")
                .parent("bar")
                .label("← Back")
                .on_press(Handler::action("leave")),
        );
        out.element(
            ui::badge("code_badge")
                .parent("bar")
                .variant("info")
                .text(cel("'Code ' + s.meeting.code")),
        );
        out.element(
            ui::badge("count_badge")
                .parent("bar")
                .text(cel("s.meeting.count")),
        );
        out.element(
            ui::text("meeting_name")
                .parent("meeting")
                .variant("h2")
                .text(cel("s.meeting.name")),
        );
        out.element(
            ui::text("meeting_desc")
                .parent("meeting")
                .visible(cel("s.meeting.description != ''"))
                .text(cel("s.meeting.description")),
        );
        out.element(
            ui::text("meeting_owner")
                .parent("meeting")
                .variant("caption")
                .text(cel("s.meeting.owner")),
        );
        out.element(
            ui::notice("closed")
                .parent("meeting")
                .variant("warning")
                .visible(cel("s.meeting.closed"))
                .text("The organizer closed this meeting: answers are locked."),
        );
        out.element(
            ui::card("answer_card")
                .parent("meeting")
                .title("Your answer")
                .gap("medium"),
        );
        out.element(
            ui::input("answer_name")
                .parent("answer_card")
                .label("Your name")
                .bind("answer.name")
                .disabled(cel("s.meeting.closed")),
        );
        out.element(
            ui::input("other")
                .parent("answer_card")
                .label("Other times that suit you (optional)")
                .bind("answer.other")
                .disabled(cel("s.meeting.closed")),
        );
        out.element(
            ui::button("save")
                .parent("answer_card")
                .label("Save my answer")
                .variant("primary")
                .disabled(cel("s.meeting.closed"))
                .on_press(Handler::action("save")),
        );
        out.element(
            ui::card("results_card")
                .parent("meeting")
                .title("Answers so far")
                .gap("small"),
        );
        out.element(
            ui::badge("best")
                .parent("results_card")
                .variant("success")
                .visible(cel("s.results.best != ''"))
                .text(cel("s.results.best")),
        );
        out.element(
            ui::list("people")
                .parent("results_card")
                .empty("No answers yet."),
        );
        out.element(
            ui::card("owner_card")
                .parent("meeting")
                .title("Organizer")
                .visible(cel("s.meeting.is_owner"))
                .gap("medium"),
        );
        out.element(
            ui::button("toggle")
                .parent("owner_card")
                .label(cel(
                    "s.meeting.closed ? 'Reopen the meeting' : 'Close the meeting'",
                ))
                .on_press(Handler::action("toggle")),
        );
        out.element(
            ui::button("delete")
                .parent("owner_card")
                .label("Delete the meeting")
                .variant("failure")
                .on_press(Handler::action("delete")),
        );
        if let Some(key) = key {
            self.show_mine(out, key);
        }
    }

    fn action(&mut self, out: &mut Outbox, action: &Action) {
        let Some(key) = key(&action.signals) else {
            return out.signal(feedback("failure", "This session has no identity."));
        };
        match action.name.as_str() {
            "create" => self.create(out, key, action),
            "join" => {
                let code = text(action, "form.code").to_uppercase();
                self.enter(out, key, &code);
            }
            "open" => {
                if let Some(code) = action.payload_as::<String>() {
                    self.enter(out, key, &code);
                }
            }
            "leave" => {
                self.go_home(out, key);
                out.signal(feedback("info", ""));
            }
            "save" => self.answer(out, key, action),
            "toggle" => {
                let owned = self.as_owner(out, key, |meetings, code| {
                    if let Some(m) = meetings.get_mut(code) {
                        m.closed = !m.closed;
                        m.changed();
                    }
                });
                if owned {
                    self.refresh(out, key);
                }
            }
            "delete" => {
                let owned = self.as_owner(out, key, |meetings, code| {
                    meetings.remove(code);
                });
                if owned {
                    self.go_home(out, key);
                    out.signal(feedback("info", "The meeting is deleted."));
                }
            }
            _ => {}
        }
    }

    fn tick(&mut self, out: &mut Outbox, _: f64) {
        let Some(code) = &self.code else { return };
        let version = MEETINGS.lock().unwrap().get(code).map(|m| m.version);
        if version == Some(self.seen) {
            return;
        }
        if let Some(key) = self.viewer {
            self.refresh(out, key);
        }
    }

    fn tick_interval(&self) -> Option<u32> {
        self.code.is_some().then_some(WATCH_MS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{connect, trigger};
    use hyperb_core::identity::Identity;
    use hypereffect::server::Loopback;

    fn person() -> PublicKey {
        Identity::generate().public()
    }

    fn create(key: PublicKey) -> (Loopback<Meet>, String) {
        let mut l = connect::<Meet>(key);
        l.write("form.title", json!("Team lunch"));
        l.write(
            "form.options",
            json!("Mon 12:00\nTue 12:30\n\nMon 12:00\nThu 13:00"),
        );
        // The organizer must give a name first.
        l.write("form.name", json!(""));
        assert!(trigger(&mut l, key, "create_btn", "onPress"));
        assert_eq!(l.client.value("view"), Some(&json!("home")));
        assert_eq!(
            l.client.attr("feedback", "text"),
            Some(json!("Enter your name."))
        );
        l.write("form.name", json!("Ann"));
        assert!(trigger(&mut l, key, "create_btn", "onPress"));
        let code = l
            .client
            .value("meeting.code")
            .unwrap()
            .as_str()
            .unwrap()
            .to_owned();
        (l, code)
    }

    #[test]
    fn create_join_answer_and_watch() {
        let (ann_key, bob_key) = (person(), person());
        let (mut ann, code) = create(ann_key);
        assert_eq!(code.len(), 6);
        assert_eq!(ann.client.value("view"), Some(&json!("meeting")));
        assert_eq!(ann.client.value("meeting.is_owner"), Some(&json!(true)));
        assert_eq!(ann.client.attr("owner_card", "visible"), Some(json!(true)));
        // Duplicates and blank lines are dropped from the times.
        assert!(ann.client.contains("opt_2") && !ann.client.contains("opt_3"));
        assert_eq!(ann.client.attr("opt_1", "label"), Some(json!("Tue 12:30")));
        assert_eq!(
            ann.client.attr("res_0", "text"),
            Some(json!("Mon 12:00: nobody yet"))
        );
        assert_eq!(
            ann.client.attr("people", "empty"),
            Some(json!("No answers yet."))
        );

        // Bob joins with the code, in lowercase.
        let mut bob = connect::<Meet>(bob_key);
        assert_eq!(bob.client.attr("mine", "visible"), Some(json!(false)));
        bob.write("form.code", json!(format!(" {} ", code.to_lowercase())));
        assert!(trigger(&mut bob, bob_key, "join_btn", "onPress"));
        assert_eq!(bob.client.value("meeting.name"), Some(&json!("Team lunch")));
        assert_eq!(
            bob.client.value("meeting.owner"),
            Some(&json!("Created by Ann"))
        );
        assert_eq!(bob.client.attr("owner_card", "visible"), Some(json!(false)));
        // Without a name there is no answer.
        assert!(trigger(&mut bob, bob_key, "save", "onPress"));
        assert_eq!(
            bob.client.attr("feedback", "variant"),
            Some(json!("failure"))
        );
        bob.write("answer.name", json!("Bob"));
        bob.write("answer.o1", json!(true));
        bob.write("answer.o2", json!(true));
        bob.write("answer.other", json!("Fri morning"));
        assert!(trigger(&mut bob, bob_key, "save", "onPress"));
        assert_eq!(
            bob.client.attr("feedback", "variant"),
            Some(json!("success"))
        );
        assert_eq!(
            bob.client.attr("res_1", "text"),
            Some(json!("Tue 12:30: 1 available — Bob (you)"))
        );

        // Ann sees it on her next tick, and adds her own answer.
        ann.tick(0.0);
        assert_eq!(
            ann.client.attr("res_1", "text"),
            Some(json!("Tue 12:30: 1 available — Bob"))
        );
        assert_eq!(
            ann.client.attr("ans_0", "text"),
            Some(json!("Bob — Tue 12:30, Thu 13:00 · also: Fri morning"))
        );
        ann.write("answer.name", json!("Ann"));
        ann.write("answer.o0", json!(true));
        ann.write("answer.o1", json!(true));
        assert!(trigger(&mut ann, ann_key, "save", "onPress"));
        assert_eq!(
            ann.client.attr("best", "text"),
            Some(json!("Best so far: Tue 12:30 (2 of 2 answered)"))
        );
        assert_eq!(
            ann.client.attr("ans_1", "text"),
            Some(json!("Ann (you) — Mon 12:00, Tue 12:30"))
        );
        bob.tick(0.0);
        assert_eq!(
            bob.client.attr("ans_1", "text"),
            Some(json!("Ann (organizer) — Mon 12:00, Tue 12:30"))
        );
        assert_eq!(
            bob.client.value("meeting.count"),
            Some(&json!("2 answered"))
        );
        assert!(ann.client.take_diagnostics().is_empty());
        assert!(bob.client.take_diagnostics().is_empty());
    }

    #[test]
    fn identities_decide_who_can_do_what() {
        let (ann_key, bob_key) = (person(), person());
        let (mut ann, code) = create(ann_key);
        let mut bob = connect::<Meet>(bob_key);
        bob.write("form.code", json!("nope00"));
        assert!(trigger(&mut bob, bob_key, "join_btn", "onPress"));
        assert_eq!(bob.client.value("view"), Some(&json!("home")));
        assert_eq!(
            bob.client.attr("feedback", "variant"),
            Some(json!("failure"))
        );
        bob.write("form.code", json!(code));
        assert!(trigger(&mut bob, bob_key, "join_btn", "onPress"));
        bob.write("answer.name", json!("Bob"));
        bob.write("answer.o0", json!(true));
        assert!(trigger(&mut bob, bob_key, "save", "onPress"));

        // Bob cannot reach the organizer's buttons, even by forging the action.
        assert!(!trigger(&mut bob, bob_key, "delete", "onPress"));
        let forged = bob
            .client
            .trigger("back", "onPress")
            .unwrap()
            .replace("\"leave\"", "\"delete\"");
        bob.server
            .receive(crate::with_key(&forged, bob_key).as_bytes());
        bob.pump();
        assert_eq!(bob.client.value("view"), Some(&json!("meeting")));
        assert_eq!(
            bob.client.attr("feedback", "text"),
            Some(json!("Only the organizer can do that."))
        );
        assert!(MEETINGS.lock().unwrap().contains_key(&code));

        // Closing locks answers for everyone.
        assert!(trigger(&mut ann, ann_key, "toggle", "onPress"));
        assert_eq!(
            ann.client.attr("toggle", "label"),
            Some(json!("Reopen the meeting"))
        );
        bob.tick(0.0);
        assert_eq!(bob.client.attr("closed", "visible"), Some(json!(true)));
        assert!(!trigger(&mut bob, bob_key, "save", "onPress"));
        assert_eq!(bob.client.attr("opt_0", "disabled"), Some(json!(true)));
        assert!(trigger(&mut ann, ann_key, "toggle", "onPress"));
        bob.tick(0.0);
        assert_eq!(bob.client.attr("save", "disabled"), Some(json!(false)));

        // Bob comes back later with the same key: the meeting is listed and
        // his answer is filled in.
        drop(bob);
        let mut bob = connect::<Meet>(bob_key);
        assert_eq!(bob.client.value("form.name"), Some(&json!("Bob")));
        assert_eq!(bob.client.attr("mine", "visible"), Some(json!(true)));
        assert_eq!(
            bob.client.attr("mine_0", "label"),
            Some(json!(format!("Team lunch ({code})")))
        );
        assert!(trigger(&mut bob, bob_key, "mine_0", "onPress"));
        assert_eq!(bob.client.value("answer.name"), Some(&json!("Bob")));
        assert_eq!(bob.client.value("answer.o0"), Some(&json!(true)));
        assert_eq!(bob.client.value("answer.o1"), Some(&json!(false)));
        // Saving again replaces his answer instead of adding one.
        bob.write("answer.o1", json!(true));
        assert!(trigger(&mut bob, bob_key, "save", "onPress"));
        assert_eq!(
            bob.client.value("meeting.count"),
            Some(&json!("1 answered"))
        );

        // The organizer deletes the meeting and Bob is told.
        assert!(trigger(&mut ann, ann_key, "delete", "onPress"));
        assert_eq!(ann.client.value("view"), Some(&json!("home")));
        assert!(!ann.client.contains("opt_0"));
        bob.tick(0.0);
        assert_eq!(bob.client.value("view"), Some(&json!("home")));
        assert_eq!(
            bob.client.attr("feedback", "variant"),
            Some(json!("warning"))
        );
        assert_eq!(bob.server.tick_interval(), None);
        assert_eq!(bob.client.attr("mine", "visible"), Some(json!(false)));
        assert!(bob.client.take_diagnostics().is_empty());
    }

    #[test]
    fn needs_a_name_and_times() {
        let key = person();
        let mut l = connect::<Meet>(key);
        l.write("form.name", json!("Ann"));
        assert!(trigger(&mut l, key, "create_btn", "onPress"));
        assert_eq!(
            l.client.attr("feedback", "text"),
            Some(json!("Give the meeting a name."))
        );
        l.write("form.title", json!("x"));
        assert!(trigger(&mut l, key, "create_btn", "onPress"));
        assert_eq!(
            l.client.attr("feedback", "text"),
            Some(json!("Propose at least one time, one per line."))
        );
        assert_eq!(l.client.value("view"), Some(&json!("home")));
    }
}
