//! The grammar checker demo, on top of Harper (`harper-core`), the same
//! engine as the Harper grammar checker.
//!
//! The text is linted on every change. Each problem gets a row with Harper's
//! message and a button per suggested fix; a fix is applied on the server and
//! the text is checked again, and the text field is rewritten through its
//! signal. "Fix all" takes the first suggestion of every problem.

use super::common::{menu, text};
use harper_core::linting::{Lint, LintGroup, Linter, Suggestion};
use harper_core::parsers::PlainEnglish;
use harper_core::spell::FstDictionary;
use harper_core::{Dialect, Document};
use hypereffect::{cel, json, ui, Action, App, Element, Handler, Map, Outbox, Value};
use std::cell::RefCell;
use std::collections::HashMap;

/// The most problems listed, and suggestions offered for each.
const MAX_ISSUES: usize = 12;
const MAX_FIXES: usize = 3;
/// The longest text checked, in characters.
const MAX_CHARS: usize = 20_000;

const SAMPLE: &str = "This is an test of the grammer checker. Their going to the \
store tomorow, and it should of been easy. Me and him was late.";

thread_local! {
    // Harper's linters are not `Send`, so each thread builds its own, once
    // per dialect, instead of the app holding one.
    static LINTERS: RefCell<HashMap<String, LintGroup>> = RefCell::new(HashMap::new());
}

fn dialect(name: &str) -> Dialect {
    match name {
        "British" => Dialect::British,
        "Canadian" => Dialect::Canadian,
        "Australian" => Dialect::Australian,
        "Indian" => Dialect::Indian,
        _ => Dialect::American,
    }
}

/// A problem found in a text, with the fixes Harper suggests.
struct Issue {
    lint: Lint,
    /// The offending text.
    shown: String,
}

fn check(source: &str, dialect_name: &str) -> Vec<Issue> {
    let doc = Document::new(source, &PlainEnglish, &FstDictionary::curated());
    let mut lints = LINTERS.with(|l| {
        l.borrow_mut()
            .entry(dialect_name.to_owned())
            .or_insert_with(|| {
                LintGroup::new_curated(FstDictionary::curated(), dialect(dialect_name))
            })
            .lint(&doc)
    });
    lints.sort_by_key(|l| (l.span.start, l.span.end));
    lints
        .into_iter()
        .map(|lint| Issue {
            shown: doc.get_span_content_str(&lint.span),
            lint,
        })
        .collect()
}

fn describe(s: &Suggestion) -> String {
    match s {
        Suggestion::ReplaceWith(c) => format!("Replace with “{}”", c.iter().collect::<String>()),
        Suggestion::InsertAfter(c) => format!("Insert “{}”", c.iter().collect::<String>()),
        Suggestion::Remove => "Remove".to_owned(),
    }
}

fn apply(source: &str, lint: &Lint, fix: &Suggestion) -> String {
    let mut chars: Vec<char> = source.chars().collect();
    fix.apply(lint.span, &mut chars);
    chars.into_iter().collect()
}

#[derive(Default)]
pub struct Grammar {
    /// Ids of the issue rows on screen.
    rows: Vec<String>,
}

impl Grammar {
    fn show(&mut self, out: &mut Outbox, source: &str, dialect_name: &str) {
        let source: String = source.chars().take(MAX_CHARS).collect();
        let issues = check(&source, dialect_name);
        let words = source.split_whitespace().count();
        out.signal(json!({"stats": {
            "words": words,
            "issues": issues.len(),
            "summary": match issues.len() {
                0 if words == 0 => "Type or paste some text.".to_owned(),
                0 => "No problems found.".to_owned(),
                1 => "1 problem found.".to_owned(),
                n => format!("{n} problems found."),
            },
        }}));
        for id in self.rows.drain(..) {
            out.remove(id);
        }
        let mut rows: Vec<Element> = Vec::new();
        for (i, issue) in issues.iter().take(MAX_ISSUES).enumerate() {
            let id = format!("issue_{i}");
            rows.push(ui::card(&id).parent("issues").gap("small"));
            rows.push(
                ui::row(format!("head_{i}"))
                    .parent(&id)
                    .gap("small")
                    .wrap(true),
            );
            rows.push(
                ui::badge(format!("kind_{i}"))
                    .parent(format!("head_{i}"))
                    .variant(match issue.lint.lint_kind.to_string().as_str() {
                        "Spelling" | "Typo" => "failure",
                        "Style" | "Readability" | "Enhancement" => "info",
                        _ => "warning",
                    })
                    .text(issue.lint.lint_kind.to_string()),
            );
            rows.push(
                ui::text(format!("word_{i}"))
                    .parent(format!("head_{i}"))
                    .variant("code")
                    .text(format!("“{}”", issue.shown)),
            );
            rows.push(
                ui::text(format!("why_{i}"))
                    .parent(&id)
                    .text(issue.lint.message.clone()),
            );
            rows.push(
                ui::row(format!("fixes_{i}"))
                    .parent(&id)
                    .gap("small")
                    .wrap(true),
            );
            for (j, fix) in issue.lint.suggestions.iter().take(MAX_FIXES).enumerate() {
                rows.push(
                    ui::button(format!("fix_{i}_{j}"))
                        .parent(format!("fixes_{i}"))
                        .label(describe(fix))
                        .variant(if j == 0 { "primary" } else { "secondary" })
                        .on_press(Handler::action("fix").payload(json!([i, j]))),
                );
            }
            self.rows.push(id);
            // Children go with their card when it is removed.
        }
        for e in rows {
            out.element(e);
        }
    }
}

impl App for Grammar {
    fn hello(&mut self, out: &mut Outbox, _: &Map<String, Value>) {
        out.signal(json!({
            "form": {"text": SAMPLE, "dialect": "American"},
            "menu": {"dialects": menu(&[
                ("American", "American English"), ("British", "British English"),
                ("Canadian", "Canadian English"), ("Australian", "Australian English"),
                ("Indian", "Indian English"),
            ])},
        }));
        out.element(ui::column("main").gap("large"));
        out.element(
            ui::text("title")
                .parent("main")
                .variant("h1")
                .text("Harpist — Grammar Checker"),
        );
        out.element(
            ui::card("input")
                .parent("main")
                .title("Your text")
                .gap("medium"),
        );
        out.element(
            ui::input("text")
                .parent("input")
                .bind("form.text")
                .variant("multiline")
                .placeholder("Write something…")
                .on_change(Handler::action("check")),
        );
        out.element(
            ui::select("dialect")
                .parent("input")
                .label("Dialect")
                .bind("form.dialect")
                .options(cel("s.menu.dialects"))
                .on_change(Handler::action("check")),
        );
        out.element(ui::row("buttons").parent("input").gap("small").wrap(true));
        out.element(
            ui::button("check")
                .parent("buttons")
                .label("Check")
                .variant("primary")
                .on_press(Handler::action("check")),
        );
        out.element(
            ui::button("fix_all")
                .parent("buttons")
                .label("Fix all")
                .disabled(cel("s.stats.issues == 0"))
                .on_press(Handler::action("fix_all")),
        );
        out.element(ui::row("badges").parent("main").gap("small").wrap(true));
        out.element(
            ui::badge("summary")
                .parent("badges")
                .variant(cel("s.stats.issues == 0 ? 'success' : 'warning'"))
                .text(cel("s.stats.summary")),
        );
        out.element(
            ui::badge("words")
                .parent("badges")
                .text(cel("string(s.stats.words) + ' words'")),
        );
        out.element(ui::column("issues").parent("main").gap("small"));
        self.show(out, SAMPLE, "American");
    }

    fn action(&mut self, out: &mut Outbox, action: &Action) {
        let source = action
            .signal("form.text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let dialect_name = text(action, "form.dialect");
        match action.name.as_str() {
            "check" => {}
            "fix" => {
                let Some([i, j]) = action.payload_as::<[usize; 2]>() else {
                    return;
                };
                let issues = check(&source, &dialect_name);
                let Some(fix) = issues.get(i).and_then(|x| x.lint.suggestions.get(j)) else {
                    return;
                };
                let fixed = apply(&source, &issues[i].lint, fix);
                out.signal(json!({"form": {"text": fixed}}));
                return self.show(out, &fixed, &dialect_name);
            }
            "fix_all" => {
                let mut fixed = source.clone();
                // From the end, so that earlier spans stay valid; overlaps are
                // left for the next round.
                let mut edge = usize::MAX;
                for issue in check(&source, &dialect_name).iter().rev() {
                    if let Some(fix) = issue.lint.suggestions.first() {
                        if issue.lint.span.end <= edge {
                            fixed = apply(&fixed, &issue.lint, fix);
                            edge = issue.lint.span.start;
                        }
                    }
                }
                out.signal(json!({"form": {"text": fixed}}));
                return self.show(out, &fixed, &dialect_name);
            }
            _ => return,
        }
        self.show(out, &source, &dialect_name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hypereffect::server::Loopback;

    #[test]
    fn finds_and_fixes_problems() {
        let mut l = Loopback::<Grammar>::new(Map::new());
        let found = l
            .client
            .value("stats.issues")
            .and_then(Value::as_u64)
            .unwrap();
        assert!(found >= 3, "found {found}");
        assert_eq!(l.client.attr("fix_all", "disabled"), Some(json!(false)));
        assert!(l.client.contains("issue_0") && l.client.contains("fix_0_0"));
        // Take the first suggestion of the first problem.
        let before = l.client.value("form.text").cloned().unwrap();
        assert!(l.trigger("fix_0_0", "onPress"));
        assert_ne!(l.client.value("form.text"), Some(&before));
        // Fixing everything leaves (almost) nothing.
        assert!(l.trigger("fix_all", "onPress"));
        let left = l
            .client
            .value("stats.issues")
            .and_then(Value::as_u64)
            .unwrap();
        assert!(left < found, "{left} < {found}");
        assert!(l.client.take_diagnostics().is_empty());
    }

    #[test]
    fn clean_and_empty_text() {
        let mut l = Loopback::<Grammar>::new(Map::new());
        l.write("form.text", json!("This is a test."));
        assert!(l.trigger("text", "onChange"));
        assert_eq!(l.client.value("stats.issues"), Some(&json!(0)));
        assert_eq!(
            l.client.attr("summary", "text"),
            Some(json!("No problems found."))
        );
        assert_eq!(l.client.attr("fix_all", "disabled"), Some(json!(true)));
        assert!(!l.client.contains("issue_0"));
        l.write("form.text", json!(""));
        assert!(l.trigger("check", "onPress"));
        assert_eq!(
            l.client.attr("summary", "text"),
            Some(json!("Type or paste some text."))
        );
        assert!(l.client.take_diagnostics().is_empty());
    }

    #[test]
    fn dialects_differ() {
        let mut l = Loopback::<Grammar>::new(Map::new());
        l.write("form.text", json!("I like the colour of the theatre."));
        l.write("form.dialect", json!("American"));
        assert!(l.trigger("dialect", "onChange"));
        let american = l.client.value("stats.issues").cloned();
        l.write("form.dialect", json!("British"));
        assert!(l.trigger("dialect", "onChange"));
        assert_eq!(l.client.value("stats.issues"), Some(&json!(0)));
        assert_ne!(american, Some(json!(0)));
    }
}
