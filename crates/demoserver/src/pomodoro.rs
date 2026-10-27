//! The pomodoro demo: a tick-driven countdown.
//!
//! Focus sessions and breaks alternate (four focus sessions, the last
//! followed by a long break). The server counts the time down on its tick and
//! patches the clock and the progress bar; when a stage ends it announces
//! it in a notice and waits, stopped, for the next start. While the timer
//! runs, the controls that would disturb it are disabled by CEL on the
//! `running` signal.

use super::common::number;
use hypereffect::{cel, json, ui, Action, App, Handler, Outbox};
use std::collections::BTreeMap;

/// How often the countdown is refreshed while running.
const TICK_MS: u32 = 250;
/// Stages in one round: focus and short break four times, the last break long.
const ROUND: usize = 8;

#[derive(Default)]
pub struct Pomodoro {
    /// Position in the round, `0..ROUND`.
    step: usize,
    focus_done: u32,
    running: bool,
    /// Seconds in the current stage, as the settings were when it began.
    total: f64,
    /// Seconds left in the current stage.
    remaining: f64,
    /// The clock of the previous tick, while running.
    last_tick: Option<f64>,
    /// Minutes for focus, short break and long break.
    minutes: [f64; 3],
}

fn is_focus(step: usize) -> bool {
    step & 1 == 0
}

fn stage_name(step: usize) -> &'static str {
    match step {
        s if is_focus(s) => "Focus",
        7 => "Long break",
        _ => "Short break",
    }
}

fn clock(secs: f64) -> String {
    let s = secs.max(0.0).ceil() as u64;
    format!("{:02}:{:02}", s / 60, s % 60)
}

impl Pomodoro {
    fn stage_secs(&self) -> f64 {
        let m = match self.step {
            s if is_focus(s) => self.minutes[0],
            7 => self.minutes[2],
            _ => self.minutes[1],
        };
        m * 60.0
    }

    fn begin_stage(&mut self, step: usize) {
        self.step = step % ROUND;
        self.total = self.stage_secs();
        self.remaining = self.total;
        self.last_tick = None;
    }

    fn untouched(&self) -> bool {
        self.remaining >= self.total
    }

    fn show(&self, out: &mut Outbox) {
        let kind = match self.step {
            s if is_focus(s) => "failure",
            7 => "info",
            _ => "success",
        };
        out.signal(json!({
            "running": self.running,
            "stage": stage_name(self.step),
            "stage_kind": kind,
            "step": format!("Stage {} of {ROUND}", self.step + 1),
            "clock": clock(self.remaining),
            "progress": if self.total > 0.0 { 1.0 - self.remaining / self.total } else { 0.0 },
            "done": self.focus_done,
        }));
    }

    fn announce(&self, out: &mut Outbox, kind: &str, text: &str) {
        out.signal(json!({"notice": {"kind": kind, "text": text}}));
    }

    fn finish(&mut self, out: &mut Outbox) {
        let ended = self.step;
        if is_focus(ended) {
            self.focus_done += 1;
        }
        self.running = false;
        self.begin_stage(ended + 1);
        let text = match ended {
            7 => "Round complete! Take a breather — start again whenever you are ready.".to_owned(),
            e if is_focus(e) => format!(
                "Focus session done. Up next: {}, {} min.",
                stage_name(self.step).to_lowercase(),
                self.stage_secs() / 60.0
            ),
            _ => format!(
                "Break over. Up next: focus, {} min.",
                self.stage_secs() / 60.0
            ),
        };
        self.show(out);
        self.announce(out, "success", &text);
    }
}

impl App for Pomodoro {
    fn hello(&mut self, out: &mut Outbox, _: &BTreeMap<String, String>) {
        self.minutes = [25.0, 5.0, 15.0];
        self.begin_stage(0);
        out.signal(json!({
            "settings": {"focus": 25, "short": 5, "long": 15},
            "notice": {"kind": "info", "text": ""},
        }));
        self.show(out);
        out.element(ui::column("main").gap("large"));
        out.element(
            ui::text("title")
                .parent("main")
                .variant("h1")
                .text("Tomato — Pomodoro Timer"),
        );
        out.element(
            ui::notice("notice")
                .parent("main")
                .variant(cel("s.notice.kind"))
                .visible(cel("s.notice.text != ''"))
                .text(cel("s.notice.text")),
        );
        out.element(ui::card("timer").parent("main").gap("medium"));
        out.element(ui::row("badges").parent("timer").gap("medium").wrap(true));
        out.element(
            ui::badge("stage")
                .parent("badges")
                .variant(cel("s.stage_kind"))
                .text(cel("s.stage")),
        );
        out.element(ui::badge("step").parent("badges").text(cel("s.step")));
        out.element(
            ui::badge("done")
                .parent("badges")
                .variant("info")
                .text(cel("string(s.done) + ' focus sessions done'")),
        );
        out.element(
            ui::text("clock")
                .parent("timer")
                .variant("h1")
                .text(cel("s.clock")),
        );
        out.element(
            ui::progress("bar")
                .parent("timer")
                .value(cel("s.progress"))
                .label(cel("s.stage + (s.running ? ' — running' : ' — paused')")),
        );
        out.element(ui::row("controls").parent("timer").gap("medium").wrap(true));
        out.element(
            ui::button("start")
                .parent("controls")
                .label(cel("s.progress > 0.0 ? 'Resume' : 'Start'"))
                .variant("primary")
                .disabled(cel("s.running"))
                .on_press(Handler::action("start")),
        );
        out.element(
            ui::button("pause")
                .parent("controls")
                .label("Pause")
                .disabled(cel("!s.running"))
                .on_press(Handler::action("pause")),
        );
        out.element(
            ui::button("reset")
                .parent("controls")
                .label("Reset stage")
                .disabled(cel("s.running"))
                .on_press(Handler::action("reset")),
        );
        out.element(
            ui::button("skip")
                .parent("controls")
                .label("Skip stage")
                .variant("plain")
                .disabled(cel("s.running"))
                .on_press(Handler::action("skip")),
        );
        out.element(
            ui::card("settings")
                .parent("main")
                .title("Durations (minutes)")
                .gap("medium"),
        );
        for (id, label, min, max, path) in [
            ("focus", "Focus", 1, 60, "settings.focus"),
            ("short", "Short break", 1, 30, "settings.short"),
            ("long", "Long break", 5, 45, "settings.long"),
        ] {
            out.element(
                ui::slider(id)
                    .parent("settings")
                    .label(label)
                    .min(min)
                    .max(max)
                    .bind(path)
                    .disabled(cel("s.running"))
                    .on_change(Handler::action("settings")),
            );
        }
    }

    fn action(&mut self, out: &mut Outbox, action: &Action) {
        match action.name.as_str() {
            "start" if !self.running => {
                self.running = true;
                self.last_tick = None;
                self.announce(out, "info", "");
                self.show(out);
            }
            "pause" if self.running => {
                self.running = false;
                self.last_tick = None;
                self.show(out);
            }
            "reset" if !self.running => {
                self.begin_stage(self.step);
                self.announce(out, "info", "");
                self.show(out);
            }
            "skip" if !self.running => {
                self.begin_stage(self.step + 1);
                self.announce(out, "info", "");
                self.show(out);
            }
            "settings" if !self.running => {
                for (i, path) in ["settings.focus", "settings.short", "settings.long"]
                    .iter()
                    .enumerate()
                {
                    if let Some(m) = number(action, path) {
                        self.minutes[i] = m.clamp(1.0, 60.0).round();
                    }
                }
                // A stage that has not begun follows the new durations.
                if self.untouched() {
                    self.begin_stage(self.step);
                    self.show(out);
                }
            }
            _ => {}
        }
    }

    fn tick(&mut self, out: &mut Outbox, now_ms: f64) {
        if !self.running {
            return;
        }
        let Some(last) = self.last_tick.replace(now_ms) else {
            return;
        };
        self.remaining -= (now_ms - last) / 1000.0;
        if self.remaining <= 0.0 {
            self.finish(out);
        } else {
            self.show(out);
        }
    }

    fn tick_interval(&self) -> Option<u32> {
        self.running.then_some(TICK_MS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hypereffect::server::Loopback;

    fn new() -> Loopback<Pomodoro> {
        Loopback::new()
    }

    #[test]
    fn counts_down_and_announces_the_end() {
        let mut l = new();
        assert_eq!(l.client.value("clock"), Some(&json!("25:00")));
        assert_eq!(l.client.attr("start", "disabled"), Some(json!(false)));
        assert_eq!(l.client.attr("pause", "disabled"), Some(json!(true)));
        assert!(l.trigger("start", "onPress"));
        // Controls lock while running.
        assert_eq!(l.client.attr("start", "disabled"), Some(json!(true)));
        assert_eq!(l.client.attr("pause", "disabled"), Some(json!(false)));
        assert_eq!(l.client.attr("reset", "disabled"), Some(json!(true)));
        assert_eq!(l.client.attr("focus", "disabled"), Some(json!(true)));
        assert_eq!(l.server.tick_interval(), Some(TICK_MS));
        l.tick(1_000.0);
        l.tick(61_000.0);
        assert_eq!(l.client.value("clock"), Some(&json!("24:00")));
        let bar = l
            .client
            .attr("bar", "value")
            .and_then(|v| v.as_f64())
            .unwrap();
        assert!((bar - 1.0 / 25.0).abs() < 1e-9);
        // The end of the stage.
        l.tick(25.0 * 60_000.0 + 2_000.0);
        assert_eq!(l.client.value("running"), Some(&json!(false)));
        assert_eq!(l.client.value("stage"), Some(&json!("Short break")));
        assert_eq!(l.client.value("clock"), Some(&json!("05:00")));
        assert_eq!(l.client.value("done"), Some(&json!(1)));
        assert_eq!(l.client.attr("notice", "visible"), Some(json!(true)));
        assert_eq!(l.client.attr("start", "disabled"), Some(json!(false)));
        assert_eq!(l.server.tick_interval(), None);
        assert!(l.client.take_diagnostics().is_empty());
    }

    #[test]
    fn pause_resume_skip_and_settings() {
        let mut l = new();
        l.write("settings.focus", json!(10));
        assert!(l.trigger("focus", "onChange"));
        assert_eq!(l.client.value("clock"), Some(&json!("10:00")));
        assert!(l.trigger("start", "onPress"));
        l.tick(0.0);
        l.tick(30_000.0);
        assert!(l.trigger("pause", "onPress"));
        assert_eq!(l.client.value("clock"), Some(&json!("09:30")));
        assert_eq!(l.client.attr("start", "label"), Some(json!("Resume")));
        // A tick while paused changes nothing.
        l.tick(90_000.0);
        assert_eq!(l.client.value("clock"), Some(&json!("09:30")));
        assert!(l.trigger("reset", "onPress"));
        assert_eq!(l.client.value("clock"), Some(&json!("10:00")));
        assert!(l.trigger("skip", "onPress"));
        assert_eq!(l.client.value("stage"), Some(&json!("Short break")));
        for _ in 0..6 {
            assert!(l.trigger("skip", "onPress"));
        }
        assert_eq!(l.client.value("stage"), Some(&json!("Long break")));
        assert_eq!(l.client.value("step"), Some(&json!("Stage 8 of 8")));
        assert!(l.client.take_diagnostics().is_empty());
    }

    #[test]
    fn running_ignores_locked_actions() {
        let mut l = new();
        assert!(l.trigger("start", "onPress"));
        // The renderer will not even send these while the buttons are disabled.
        assert!(!l.trigger("skip", "onPress"));
        assert!(!l.trigger("start", "onPress"));
    }
}
