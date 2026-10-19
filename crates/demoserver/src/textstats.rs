//! The text stats demo: paste text, get numbers.
//!
//! The client pastes or types text into a multiline field and presses
//! "Analyze"; the server answers with the word count, the character count,
//! the number of unique words and the five most used words. This one holds
//! no memory between sessions.

use hypereffect::{json, ui, Action, App, Handler, Map, Outbox, Value};

/// How many top words to rank.
const TOP: usize = 5;

#[derive(Default)]
pub struct TextStats;

fn analyze(text: &str) -> Value {
    let words: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_lowercase())
        .collect();
    let mut counts = std::collections::HashMap::new();
    for word in &words {
        *counts.entry(word.as_str()).or_insert(0) += 1;
    }
    let mut ranked: Vec<(&str, usize)> = counts.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    let top: Vec<Value> = ranked
        .iter()
        .take(TOP)
        .map(|(word, count)| json!({"word": *word, "count": *count}))
        .collect();
    json!({
        "words": words.len(),
        "chars": text.chars().count(),
        "unique": ranked.len(),
        "top": top,
    })
}

impl App for TextStats {
    fn hello(&mut self, out: &mut Outbox, _: &Map<String, Value>) {
        out.signal(json!({
            "stats": {"words": 0, "chars": 0, "unique": 0, "top": []},
            "feedback": {"kind": "info", "text": ""},
        }));
        out.element(ui::column("main").gap("large"));
        out.element(
            ui::text("title")
                .parent("main")
                .variant("h1")
                .text("Wordlare — Text Stats"),
        );
        out.element(
            ui::card("paste")
                .parent("main")
                .title("Your text")
                .gap("medium"),
        );
        out.element(
            ui::input("source")
                .parent("paste")
                .bind("form.text")
                .variant("multiline")
                .placeholder("Paste text here…"),
        );
        out.element(
            ui::button("analyze")
                .parent("paste")
                .label("Analyze")
                .variant("primary")
                .on_press(Handler::action("analyze")),
        );
        out.element(
            ui::card("results")
                .parent("main")
                .title("Results")
                .gap("medium"),
        );
        out.element(ui::row("counts").parent("results").gap("medium").wrap(true));
        out.element(
            ui::badge("words")
                .parent("counts")
                .variant("info")
                .text(json!({"cel": "string(s.stats.words) + ' words'"})),
        );
        out.element(
            ui::badge("chars")
                .parent("counts")
                .variant("info")
                .text(json!({"cel": "string(s.stats.chars) + ' characters'"})),
        );
        out.element(
            ui::badge("unique")
                .parent("counts")
                .variant("success")
                .text(json!({"cel": "string(s.stats.unique) + ' unique words'"})),
        );
        out.element(
            ui::list("top")
                .parent("results")
                .empty("Nothing to rank yet."),
        );
        for i in 0..TOP {
            out.element(
                ui::badge(format!("rank{i}"))
                    .parent("top")
                    .visible(json!({ "cel": format!("size(s.stats.top) > {i}") }))
                    .text(json!({
                        "cel":
                            format!("'{i}. ' + s.stats.top[{i}].word + ' — ' + string(s.stats.top[{i}].count) + '×'")
                    })),
            );
        }
        out.element(
            ui::notice("feedback")
                .parent("main")
                .variant(json!({"cel": "s.feedback.kind"}))
                .visible(json!({"cel": "s.feedback.text != ''"}))
                .text(json!({"cel": "s.feedback.text"})),
        );
    }

    fn action(&mut self, out: &mut Outbox, action: &Action) {
        if action.name != "analyze" {
            return;
        }
        let text = action
            .signal("form.text")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if text.trim().is_empty() {
            out.signal(json!({"feedback": {
                "kind": "failure",
                "text": "Paste some text first.",
            }}));
            return;
        }
        out.signal(json!({"stats": analyze(text), "feedback": {"kind": "info", "text": ""}}));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hypereffect::server::Loopback;

    #[test]
    fn counts_words() {
        let mut l = Loopback::<TextStats>::new();
        l.write("form.text", json!("rose rose tulip fern"));
        assert!(l.trigger("analyze", "onPress"));
        assert_eq!(l.client.value("stats.words"), Some(&json!(4)));
        assert_eq!(l.client.value("stats.chars"), Some(&json!(20)));
        assert_eq!(l.client.value("stats.unique"), Some(&json!(3)));
        assert_eq!(
            l.client.value("stats.top"),
            Some(&json!([
                {"word": "rose", "count": 2},
                {"word": "fern", "count": 1},
                {"word": "tulip", "count": 1}
            ]))
        );
        assert_eq!(l.client.attr("rank0", "text"), Some(json!("0. rose — 2×")));
        assert_eq!(l.client.attr("rank1", "visible"), Some(json!(true)));
        assert_eq!(l.client.attr("rank3", "visible"), Some(json!(false)));
        assert!(l.client.take_diagnostics().is_empty());
    }

    #[test]
    fn empty_text_is_rejected() {
        let mut l = Loopback::<TextStats>::new();
        assert!(l.trigger("analyze", "onPress"));
        assert_eq!(
            l.client.attr("feedback", "text"),
            Some(json!("Paste some text first."))
        );
        assert!(l.client.take_diagnostics().is_empty());
    }

    #[test]
    fn non_latin_text() {
        let mut l = Loopback::<TextStats>::new();
        l.write("form.text", json!("∀∃! 희망, 희망!"));
        assert!(l.trigger("analyze", "onPress"));
        assert_eq!(l.client.value("stats.words"), Some(&json!(2)));
        assert_eq!(l.client.value("stats.unique"), Some(&json!(1)));
        assert!(l.client.take_diagnostics().is_empty());
    }
}
