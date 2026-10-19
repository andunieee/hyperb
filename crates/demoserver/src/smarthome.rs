//! The smart home demo: a monitor and control panel for a fictional house.
//! Nothing is connected to anything.
//!
//! The house is simulated on the tick (one second is three minutes of house
//! time): the thermostat heats or cools the rooms against the weather, the
//! garage door travels, the cameras see things, and the power draw is
//! charted. Controls are ordinary bound signals that the server reads from
//! each action; scenes and automations change them from the server.

use super::chart::{draw, Point, PALETTE};
use super::common::{menu, number, replace, text};
use super::img::{self, Canvas};
use hypereffect::{cel, json, ui, Action, App, Element, Handler, Map, Outbox, Value};

const TICK_MS: u32 = 1000;
/// House minutes per tick.
const MINUTES_PER_TICK: u32 = 3;
const HISTORY: usize = 24;
const ALERTS: usize = 10;
const LIGHTS: [(&str, &str); 4] = [
    ("living", "Living room"),
    ("kitchen", "Kitchen"),
    ("bedroom", "Bedroom"),
    ("porch", "Porch"),
];
const CAMERAS: [(&str, &str); 2] = [("front", "Front door"), ("garden", "Garden")];
const AUTOMATIONS: [(&str, &str); 5] = [
    ("dusk", "Porch light on at 19:00, off at 06:00"),
    ("lock", "Lock both doors at 22:30"),
    ("garage", "Close the garage at 23:00"),
    ("setback", "Night setback: 3° cooler from 23:00 to 06:00"),
    (
        "window",
        "Warn when a window is open while heating or cooling",
    ),
];

#[derive(Clone)]
struct Light {
    on: bool,
    level: u32,
    warm: String,
}

#[derive(Clone, Copy, PartialEq)]
enum Garage {
    Closed,
    Opening(f64),
    Open,
    Closing(f64),
}

struct Rng(u64);

impl Rng {
    fn next(&mut self, below: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) % below
    }
}

pub struct SmartHome {
    /// Minutes since midnight, and days elapsed.
    minute: u32,
    day: u32,
    lights: Vec<Light>,
    target: f64,
    mode: String,
    indoor: f64,
    locks: [bool; 2],
    windows: [bool; 2],
    garage: Garage,
    armed: [bool; 2],
    motion: [u32; 2],
    away: bool,
    scene: &'static str,
    automations: Vec<bool>,
    /// The target before the night setback, while it is in force.
    stashed_target: Option<f64>,
    last_window_alert: Option<u32>,
    watts: f64,
    kwh: f64,
    history: Vec<(u32, f64)>,
    alerts: Vec<String>,
    rng: Rng,
    ticks: u64,
    /// Ids of the per-device power rows.
    power_ids: Vec<String>,
}

impl Default for SmartHome {
    fn default() -> Self {
        SmartHome {
            minute: 18 * 60,
            day: 1,
            lights: vec![
                Light {
                    on: true,
                    level: 70,
                    warm: "warm".into(),
                },
                Light {
                    on: false,
                    level: 100,
                    warm: "neutral".into(),
                },
                Light {
                    on: false,
                    level: 40,
                    warm: "warm".into(),
                },
                Light {
                    on: false,
                    level: 60,
                    warm: "neutral".into(),
                },
            ],
            target: 21.0,
            mode: "heat".into(),
            indoor: 19.5,
            locks: [true, true],
            windows: [false, false],
            garage: Garage::Closed,
            armed: [false, false],
            motion: [0, 0],
            away: false,
            scene: "home",
            automations: vec![true, true, true, false, true],
            stashed_target: None,
            last_window_alert: None,
            watts: 0.0,
            kwh: 0.0,
            history: Vec::new(),
            alerts: Vec::new(),
            rng: Rng(11),
            ticks: 0,
            power_ids: Vec::new(),
        }
    }
}

fn clock(minute: u32) -> String {
    format!("{:02}:{:02}", minute / 60 % 24, minute % 60)
}

impl SmartHome {
    fn outdoor(&self) -> f64 {
        // Coldest at 04:00, warmest at 16:00.
        let h = f64::from(self.minute) / 60.0;
        8.0 + 6.0 * ((h - 10.0) / 24.0 * std::f64::consts::TAU).sin()
    }

    fn is_dark(&self) -> bool {
        self.minute < 6 * 60 || self.minute >= 19 * 60
    }

    fn note(&mut self, line: impl AsRef<str>) {
        self.alerts
            .push(format!("{} · {}", clock(self.minute), line.as_ref()));
        if self.alerts.len() > ALERTS {
            self.alerts.remove(0);
        }
    }

    fn hvac_active(&self) -> bool {
        match self.mode.as_str() {
            "heat" => self.indoor < self.target - 0.2,
            "cool" => self.indoor > self.target + 0.2,
            "auto" => (self.indoor - self.target).abs() > 0.2,
            _ => false,
        }
    }

    /// The power each part of the house draws, in watts.
    fn draws(&self) -> Vec<(&'static str, f64)> {
        let lights: f64 = self
            .lights
            .iter()
            .filter(|l| l.on)
            .map(|l| 40.0 * f64::from(l.level) / 100.0)
            .sum();
        let garage = if matches!(self.garage, Garage::Opening(_) | Garage::Closing(_)) {
            350.0
        } else {
            4.0
        };
        let cameras = 6.0 * self.armed.iter().filter(|a| **a).count() as f64 + 4.0;
        vec![
            ("Lights", lights),
            (
                "Heating & cooling",
                if self.hvac_active() { 1800.0 } else { 5.0 },
            ),
            ("Garage door", garage),
            ("Cameras & sensors", cameras),
            ("Everything else", 90.0),
        ]
    }

    /// Reads the controls back from an action's signals, noting changes.
    fn absorb(&mut self, action: &Action) {
        for (i, (id, name)) in LIGHTS.iter().enumerate() {
            let on = action
                .signal(&format!("light.{id}.on"))
                .and_then(Value::as_bool);
            if let Some(on) = on {
                if on != self.lights[i].on {
                    self.note(format!(
                        "{name} light turned {}",
                        if on { "on" } else { "off" }
                    ));
                }
                self.lights[i].on = on;
            }
            if let Some(l) = number(action, &format!("light.{id}.level")) {
                self.lights[i].level = l.clamp(1.0, 100.0) as u32;
            }
            let warm = text(action, &format!("light.{id}.warm"));
            if !warm.is_empty() {
                self.lights[i].warm = warm;
            }
        }
        if let Some(t) = number(action, "climate.target") {
            let t = (t.clamp(16.0, 28.0) * 2.0).round() / 2.0;
            if (t - self.target).abs() > 1e-9 {
                self.note(format!("Thermostat set to {t:.1}°"));
            }
            self.target = t;
        }
        let mode = text(action, "climate.mode");
        if !mode.is_empty() && mode != self.mode {
            self.note(format!("Thermostat mode: {mode}"));
            self.mode = mode;
        }
        for (i, name) in ["front", "back"].iter().enumerate() {
            if let Some(l) = action
                .signal(&format!("lock.{name}"))
                .and_then(Value::as_bool)
            {
                if l != self.locks[i] {
                    self.note(format!(
                        "The {name} door was {}",
                        if l { "locked" } else { "unlocked" }
                    ));
                }
                self.locks[i] = l;
            }
        }
        for (i, name) in ["kitchen", "bedroom"].iter().enumerate() {
            if let Some(o) = action
                .signal(&format!("window.{name}"))
                .and_then(Value::as_bool)
            {
                if o != self.windows[i] {
                    self.note(format!(
                        "The {name} window was {}",
                        if o { "opened" } else { "closed" }
                    ));
                }
                self.windows[i] = o;
            }
        }
        for (i, (id, name)) in CAMERAS.iter().enumerate() {
            if let Some(a) = action
                .signal(&format!("cam.{id}.armed"))
                .and_then(Value::as_bool)
            {
                if a != self.armed[i] {
                    self.note(format!(
                        "{name} camera {}",
                        if a { "armed" } else { "disarmed" }
                    ));
                }
                self.armed[i] = a;
            }
        }
        for (i, (id, _)) in AUTOMATIONS.iter().enumerate() {
            if let Some(a) = action
                .signal(&format!("auto.{id}"))
                .and_then(Value::as_bool)
            {
                self.automations[i] = a;
            }
        }
    }

    fn scene(&mut self, name: &str) {
        let set = |s: &mut Self, living: Option<u32>, bedroom: Option<u32>, target: f64| {
            for (i, level) in [(0, living), (2, bedroom)] {
                s.lights[i].on = level.is_some();
                if let Some(l) = level {
                    s.lights[i].level = l;
                    s.lights[i].warm = "warm".into();
                }
            }
            s.lights[1].on = false;
            s.lights[3].on = false;
            s.target = target;
            s.mode = "heat".into();
        };
        match name {
            "home" => {
                set(self, Some(70), None, 21.0);
                self.armed = [false, false];
                self.away = false;
            }
            "away" => {
                set(self, None, None, 17.0);
                self.locks = [true, true];
                self.armed = [true, true];
                if self.garage != Garage::Closed {
                    self.garage = Garage::Closing(0.0);
                }
                self.away = true;
            }
            "night" => {
                set(self, None, Some(10), 18.5);
                self.locks = [true, true];
                self.armed = [true, true];
                self.away = false;
            }
            "movie" => {
                set(self, Some(15), None, 21.0);
                self.away = false;
            }
            _ => return,
        }
        self.scene = match name {
            "away" => "away",
            "night" => "night",
            "movie" => "movie",
            _ => "home",
        };
        self.stashed_target = None;
        self.note(format!("Scene: {name}"));
    }

    /// Whether the clock passed `at` (a minute of the day) in the last step.
    fn crossed(&self, before: u32, at: u32) -> bool {
        let now = self.minute;
        if before <= now {
            before < at && at <= now
        } else {
            at > before || at <= now
        }
    }

    /// Advances the house by one tick. Returns whether controls were changed.
    fn step(&mut self) -> bool {
        let before = self.minute;
        self.minute += MINUTES_PER_TICK;
        if self.minute >= 24 * 60 {
            self.minute -= 24 * 60;
            self.day += 1;
        }
        self.ticks += 1;
        let mut changed = false;

        // Weather and the thermostat.
        let leak = if self.windows.iter().any(|w| *w) {
            0.06
        } else {
            0.012
        };
        self.indoor += (self.outdoor() - self.indoor) * leak;
        if self.hvac_active() {
            let up = match self.mode.as_str() {
                "heat" => true,
                "cool" => false,
                _ => self.indoor < self.target,
            };
            self.indoor += if up { 0.4 } else { -0.4 };
        }

        // The garage door.
        self.garage = match self.garage {
            Garage::Opening(p) if p + 0.25 >= 1.0 => Garage::Open,
            Garage::Opening(p) => Garage::Opening(p + 0.25),
            Garage::Closing(p) if p + 0.25 >= 1.0 => Garage::Closed,
            Garage::Closing(p) => Garage::Closing(p + 0.25),
            g => g,
        };

        // Automations.
        if self.automations[0] {
            if self.crossed(before, 19 * 60) {
                self.lights[3].on = true;
                self.note("Porch light on at dusk");
                changed = true;
            }
            if self.crossed(before, 6 * 60) {
                self.lights[3].on = false;
                self.note("Porch light off at dawn");
                changed = true;
            }
        }
        if self.automations[1] && self.crossed(before, 22 * 60 + 30) && self.locks != [true, true] {
            self.locks = [true, true];
            self.note("Doors locked for the night");
            changed = true;
        }
        if self.automations[2] && self.crossed(before, 23 * 60) && self.garage != Garage::Closed {
            self.garage = Garage::Closing(0.0);
            self.note("Closing the garage door for the night");
        }
        if self.automations[3] {
            if self.crossed(before, 23 * 60) && self.stashed_target.is_none() {
                self.stashed_target = Some(self.target);
                self.target = (self.target - 3.0).max(16.0);
                self.note(format!("Night setback: thermostat at {:.1}°", self.target));
                changed = true;
            }
            if self.crossed(before, 6 * 60) {
                if let Some(t) = self.stashed_target.take() {
                    self.target = t;
                    self.note(format!("Morning: thermostat back to {t:.1}°"));
                    changed = true;
                }
            }
        }
        let open = self.windows.iter().any(|w| *w);
        if self.automations[4] && open && self.mode != "off" {
            let quiet = self
                .last_window_alert
                .is_some_and(|t| t + 30 > self.ticks as u32 * MINUTES_PER_TICK);
            if !quiet {
                self.last_window_alert = Some(self.ticks as u32 * MINUTES_PER_TICK);
                self.note("⚠ A window is open while the thermostat is running");
            }
        }

        // The cameras notice things, more so at night.
        for (i, (_, camera)) in CAMERAS.iter().enumerate() {
            let odds = if self.is_dark() { 7 } else { 3 };
            if self.armed[i] && self.rng.next(100) < odds {
                self.motion[i] += 1;
                let what =
                    ["a cat", "a person", "a delivery driver", "a fox"][self.rng.next(4) as usize];
                self.note(format!(
                    "Motion at the {} camera: {what}",
                    camera.to_lowercase()
                ));
                if self.away {
                    self.note("⚠ Motion while nobody is home");
                }
            }
        }

        // Power.
        self.watts = self.draws().iter().map(|(_, w)| w).sum();
        self.kwh += self.watts * f64::from(MINUTES_PER_TICK) / 60.0 / 1000.0;
        self.history.push((self.minute, self.watts));
        if self.history.len() > HISTORY {
            self.history.remove(0);
        }
        changed
    }

    // --- display -----------------------------------------------------------

    fn snapshot(&self, i: usize, motion: bool) -> String {
        let mut c = Canvas::new(320, 180, [0, 0, 0]);
        let h = self.minute / 60;
        let sky = match h {
            6..=7 | 18 => [240, 170, 110],
            8..=17 => [150, 200, 240],
            _ => [18, 22, 46],
        };
        c.rect(0, 0, 320, 180, sky);
        let ground = if self.is_dark() {
            [18, 40, 26]
        } else {
            [70, 140, 70]
        };
        c.rect(0, 120, 320, 60, ground);
        if i == 0 {
            // The house front: wall, door, a lit window.
            c.rect(80, 50, 160, 90, [150, 110, 80]);
            c.rect(70, 40, 180, 14, [90, 50, 40]);
            c.rect(145, 80, 30, 60, [70, 40, 30]);
            let lit = self.lights[0].on;
            c.rect(
                100,
                70,
                30,
                30,
                if lit { [250, 220, 120] } else { [60, 70, 90] },
            );
            c.rect(190, 70, 30, 30, [60, 70, 90]);
            if self.lights[3].on {
                c.disc(160, 62, 6, [255, 240, 160]);
            }
        } else {
            for (x, r) in [(40, 22), (110, 30), (230, 26), (290, 20)] {
                c.disc(x, 118, r, [30, 100, 40]);
            }
            c.rect(150, 60, 8, 60, [90, 60, 40]);
            c.disc(154, 56, 30, [40, 120, 50]);
        }
        if motion {
            c.rect(205, 95, 20, 4, [220, 40, 40]);
            c.rect(205, 95, 4, 45, [220, 40, 40]);
            c.rect(221, 95, 4, 45, [220, 40, 40]);
            c.rect(205, 136, 20, 4, [220, 40, 40]);
            c.disc(215, 108, 6, [30, 30, 30]);
            c.rect(210, 114, 10, 20, [30, 30, 30]);
            c.text(205, 82, 1, "MOTION", [255, 80, 80]);
        }
        c.rect(0, 0, 320, 14, [0, 0, 0]);
        c.text(
            4,
            4,
            1,
            &format!("{} {}", CAMERAS[i].1, clock(self.minute)),
            [255, 255, 255],
        );
        if self.armed[i] {
            c.disc(308, 7, 3, [230, 40, 40]);
        }
        img::data_uri(&c.png())
    }

    fn show_cameras(&self, out: &mut Outbox, motion: [bool; 2]) {
        for (i, (id, _)) in CAMERAS.iter().enumerate() {
            out.signal(json!({"cam": {*id: {
                "src": self.snapshot(i, motion[i]),
                "events": format!("{} motion event{}", self.motion[i], if self.motion[i] == 1 { "" } else { "s" }),
            }}}));
        }
    }

    fn controls(&self, out: &mut Outbox) {
        let mut lights = Map::new();
        for (i, (id, _)) in LIGHTS.iter().enumerate() {
            let l = &self.lights[i];
            lights.insert(
                (*id).into(),
                json!({"on": l.on, "level": l.level, "warm": l.warm}),
            );
        }
        out.signal(json!({
            "light": lights,
            "climate": {"target": self.target, "mode": self.mode},
            "lock": {"front": self.locks[0], "back": self.locks[1]},
            "window": {"kitchen": self.windows[0], "bedroom": self.windows[1]},
            "cam": {"front": {"armed": self.armed[0]}, "garden": {"armed": self.armed[1]}},
        }));
    }

    fn status(&mut self, out: &mut Outbox) {
        let (garage, progress) = match self.garage {
            Garage::Closed => ("closed", 0.0),
            Garage::Open => ("open", 1.0),
            Garage::Opening(p) => ("opening", p),
            Garage::Closing(p) => ("closing", 1.0 - p),
        };
        let active = if self.hvac_active() {
            if self.indoor < self.target {
                "heating"
            } else {
                "cooling"
            }
        } else {
            "idle"
        };
        let draws = self.draws();
        let mut alerts: Vec<String> = self.alerts.iter().rev().cloned().collect();
        alerts.resize(ALERTS, String::new());
        let open_windows = self.windows.iter().filter(|w| **w).count();
        out.signal(json!({
            "home": {
                "time": format!("Day {}, {}", self.day, clock(self.minute)),
                "outdoor": format!("Outside {:.1}°C", self.outdoor()),
                "indoor": format!("Inside {:.1}°C", self.indoor),
                "scene": self.scene,
                "doors": if self.locks == [true, true] { "All doors locked" } else { "A door is unlocked" },
                "doors_kind": if self.locks == [true, true] { "success" } else { "warning" },
                "windows": if open_windows == 0 { "Windows closed".to_owned() } else { format!("{open_windows} window(s) open") },
                "windows_kind": if open_windows == 0 { "success" } else { "warning" },
                "garage": format!("Garage {garage}"),
                "hvac": format!("Thermostat {active}"),
            },
            "garage": {"state": garage, "progress": progress},
            "energy": {
                "now": format!("{:.0} W now", self.watts),
                "today": format!("{:.2} kWh today", self.kwh),
            },
            "alerts": alerts,
        }));
        let rows: Vec<Element> = draws
            .iter()
            .enumerate()
            .flat_map(|(i, (name, w))| {
                [ui::text(format!("draw_{i}"))
                    .parent("draws")
                    .text(format!("{name}: {w:.0} W"))]
            })
            .collect();
        replace(out, &mut self.power_ids, rows);
    }

    fn chart(&self, out: &mut Outbox) {
        let points: Vec<Point> = self
            .history
            .iter()
            .map(|(m, w)| Point {
                label: clock(*m),
                value: w.round(),
            })
            .collect();
        let src = if points.len() >= 2 {
            draw("line", "Power draw (W)", &points, PALETTE[1])
                .map(|p| img::data_uri(&p))
                .unwrap_or_default()
        } else {
            String::new()
        };
        out.signal(json!({"energy": {"chart": src}}));
    }
}

impl App for SmartHome {
    fn hello(&mut self, out: &mut Outbox, _: &Map<String, Value>) {
        self.watts = self.draws().iter().map(|(_, w)| w).sum();
        self.history.push((self.minute, self.watts));
        self.note("The house is online");
        out.signal(json!({
            "menu": {
                "modes": menu(&[("heat", "Heat"), ("cool", "Cool"), ("auto", "Auto"), ("off", "Off")]),
                "warmth": menu(&[("warm", "Warm"), ("neutral", "Neutral"), ("cool", "Cool white")]),
            },
            "auto": {"dusk": true, "lock": true, "garage": true, "setback": false, "window": true},
            "energy": {"chart": ""},
        }));
        self.controls(out);
        out.element(ui::column("main").gap("large"));
        out.element(
            ui::text("title")
                .parent("main")
                .variant("h1")
                .text("Hearth — Smart Home"),
        );
        out.element(ui::row("summary").parent("main").gap("small").wrap(true));
        out.element(
            ui::badge("b_time")
                .parent("summary")
                .variant("info")
                .text(cel("s.home.time")),
        );
        out.element(
            ui::badge("b_out")
                .parent("summary")
                .text(cel("s.home.outdoor")),
        );
        out.element(
            ui::badge("b_in")
                .parent("summary")
                .text(cel("s.home.indoor")),
        );
        out.element(
            ui::badge("b_doors")
                .parent("summary")
                .variant(cel("s.home.doors_kind"))
                .text(cel("s.home.doors")),
        );
        out.element(
            ui::badge("b_windows")
                .parent("summary")
                .variant(cel("s.home.windows_kind"))
                .text(cel("s.home.windows")),
        );
        out.element(
            ui::badge("b_power")
                .parent("summary")
                .variant("info")
                .text(cel("s.energy.now")),
        );
        out.element(ui::tabs("tabs").parent("main"));

        // Overview.
        out.element(ui::tab("overview").parent("tabs").label("Overview"));
        out.element(
            ui::text("scenes_title")
                .parent("overview")
                .variant("h3")
                .text("Scenes"),
        );
        out.element(ui::row("scenes").parent("overview").gap("small").wrap(true));
        for (id, label) in [
            ("home", "Home"),
            ("away", "Away"),
            ("night", "Night"),
            ("movie", "Movie"),
        ] {
            out.element(
                ui::button(format!("scene_{id}"))
                    .parent("scenes")
                    .label(label)
                    .variant(cel(format!(
                        "s.home.scene == '{id}' ? 'primary' : 'secondary'"
                    )))
                    .on_press(Handler::action("scene").payload(id)),
            );
        }
        out.element(
            ui::text("alerts_title")
                .parent("overview")
                .variant("h3")
                .text("Activity"),
        );
        for k in 0..ALERTS {
            out.element(
                ui::text(format!("alert_{k}"))
                    .parent("overview")
                    .variant("code")
                    .visible(cel(format!("s.alerts[{k}] != ''")))
                    .text(cel(format!("s.alerts[{k}]"))),
            );
        }

        // Lights.
        out.element(ui::tab("lights").parent("tabs").label("Lights"));
        for (id, name) in LIGHTS {
            out.element(
                ui::card(format!("card_{id}"))
                    .parent("lights")
                    .title(name)
                    .gap("small"),
            );
            out.element(
                ui::checkbox(format!("on_{id}"))
                    .parent(format!("card_{id}"))
                    .label("On")
                    .variant("switch")
                    .bind(format!("light.{id}.on"))
                    .on_change(Handler::action("sync")),
            );
            out.element(
                ui::slider(format!("level_{id}"))
                    .parent(format!("card_{id}"))
                    .label("Brightness")
                    .min(1)
                    .max(100)
                    .bind(format!("light.{id}.level"))
                    .disabled(cel(format!("!s.light.{id}.on")))
                    .on_change(Handler::action("sync")),
            );
            out.element(
                ui::select(format!("warm_{id}"))
                    .parent(format!("card_{id}"))
                    .label("Color")
                    .variant("segmented")
                    .bind(format!("light.{id}.warm"))
                    .options(cel("s.menu.warmth"))
                    .disabled(cel(format!("!s.light.{id}.on")))
                    .on_change(Handler::action("sync")),
            );
        }

        // Climate.
        out.element(ui::tab("climate").parent("tabs").label("Climate"));
        out.element(ui::row("temps").parent("climate").gap("small").wrap(true));
        out.element(
            ui::badge("t_in")
                .parent("temps")
                .variant("info")
                .text(cel("s.home.indoor")),
        );
        out.element(
            ui::badge("t_state")
                .parent("temps")
                .variant(cel(
                    "s.home.hvac == 'Thermostat idle' ? 'neutral' : 'warning'",
                ))
                .text(cel("s.home.hvac")),
        );
        out.element(
            ui::select("mode")
                .parent("climate")
                .label("Mode")
                .variant("segmented")
                .bind("climate.mode")
                .options(cel("s.menu.modes"))
                .on_change(Handler::action("sync")),
        );
        out.element(
            ui::slider("target")
                .parent("climate")
                .label("Target temperature (°C)")
                .min(16)
                .max(28)
                .step(0.5)
                .bind("climate.target")
                .disabled(cel("s.climate.mode == 'off'"))
                .on_change(Handler::action("sync")),
        );
        out.element(
            ui::text("target_text")
                .parent("climate")
                .text(cel("'Target ' + string(s.climate.target) + '°C'")),
        );
        out.element(
            ui::checkbox("w_kitchen")
                .parent("climate")
                .label("Kitchen window open")
                .bind("window.kitchen")
                .on_change(Handler::action("sync")),
        );
        out.element(
            ui::checkbox("w_bedroom")
                .parent("climate")
                .label("Bedroom window open")
                .bind("window.bedroom")
                .on_change(Handler::action("sync")),
        );

        // Security.
        out.element(ui::tab("security").parent("tabs").label("Security"));
        out.element(
            ui::checkbox("lock_front")
                .parent("security")
                .label("Front door locked")
                .variant("switch")
                .bind("lock.front")
                .on_change(Handler::action("sync")),
        );
        out.element(
            ui::checkbox("lock_back")
                .parent("security")
                .label("Back door locked")
                .variant("switch")
                .bind("lock.back")
                .on_change(Handler::action("sync")),
        );
        out.element(
            ui::card("garage_card")
                .parent("security")
                .title("Garage door")
                .gap("small"),
        );
        out.element(
            ui::progress("garage_bar")
                .parent("garage_card")
                .value(cel("s.garage.progress"))
                .label(cel("s.home.garage")),
        );
        out.element(ui::row("garage_buttons").parent("garage_card").gap("small"));
        out.element(
            ui::button("garage_open")
                .parent("garage_buttons")
                .label("Open")
                .disabled(cel("s.garage.state != 'closed'"))
                .on_press(Handler::action("garage").payload("open")),
        );
        out.element(
            ui::button("garage_close")
                .parent("garage_buttons")
                .label("Close")
                .disabled(cel("s.garage.state != 'open'"))
                .on_press(Handler::action("garage").payload("close")),
        );
        for (id, name) in CAMERAS {
            out.element(
                ui::card(format!("cam_{id}"))
                    .parent("security")
                    .title(name)
                    .gap("small"),
            );
            out.element(
                ui::image(format!("shot_{id}"))
                    .parent(format!("cam_{id}"))
                    .alt(format!("{name} camera"))
                    .src(cel(format!("s.cam.{id}.src"))),
            );
            out.element(
                ui::row(format!("cam_{id}_row"))
                    .parent(format!("cam_{id}"))
                    .gap("small")
                    .wrap(true),
            );
            out.element(
                ui::checkbox(format!("arm_{id}"))
                    .parent(format!("cam_{id}_row"))
                    .label("Armed")
                    .variant("switch")
                    .bind(format!("cam.{id}.armed"))
                    .on_change(Handler::action("sync")),
            );
            out.element(
                ui::badge(format!("events_{id}"))
                    .parent(format!("cam_{id}_row"))
                    .text(cel(format!("s.cam.{id}.events"))),
            );
            out.element(
                ui::button(format!("refresh_{id}"))
                    .parent(format!("cam_{id}_row"))
                    .label("Refresh")
                    .variant("plain")
                    .on_press(Handler::action("snapshot").payload(id)),
            );
        }

        // Energy.
        out.element(ui::tab("energy").parent("tabs").label("Energy"));
        out.element(ui::row("energy_badges").parent("energy").gap("small"));
        out.element(
            ui::badge("e_now")
                .parent("energy_badges")
                .variant("info")
                .text(cel("s.energy.now")),
        );
        out.element(
            ui::badge("e_today")
                .parent("energy_badges")
                .text(cel("s.energy.today")),
        );
        out.element(
            ui::image("power_chart")
                .parent("energy")
                .alt("Power draw over time")
                .visible(cel("s.energy.chart != ''"))
                .src(cel("s.energy.chart")),
        );
        out.element(ui::list("draws").parent("energy"));

        // Automations.
        out.element(ui::tab("automations").parent("tabs").label("Automations"));
        out.element(
            ui::text("auto_help")
                .parent("automations")
                .variant("caption")
                .text("The house clock runs fast: a day takes about 8 minutes."),
        );
        for (id, label) in AUTOMATIONS {
            out.element(
                ui::checkbox(format!("auto_{id}"))
                    .parent("automations")
                    .label(label)
                    .variant("switch")
                    .bind(format!("auto.{id}"))
                    .on_change(Handler::action("sync")),
            );
        }

        self.show_cameras(out, [false, false]);
        self.chart(out);
        self.status(out);
    }

    fn action(&mut self, out: &mut Outbox, action: &Action) {
        self.absorb(action);
        match action.name.as_str() {
            "scene" => {
                let name: String = action.payload_as().unwrap_or_default();
                self.scene(&name);
                self.controls(out);
            }
            "garage" => match (action.payload_as::<String>().as_deref(), self.garage) {
                (Some("open"), Garage::Closed) => {
                    self.garage = Garage::Opening(0.0);
                    self.note("Garage door opening");
                }
                (Some("close"), Garage::Open) => {
                    self.garage = Garage::Closing(0.0);
                    self.note("Garage door closing");
                }
                _ => {}
            },
            "snapshot" => {
                let id: String = action.payload_as().unwrap_or_default();
                if let Some(i) = CAMERAS.iter().position(|(c, _)| *c == id) {
                    out.signal(json!({"cam": {id.as_str(): {"src": self.snapshot(i, false)}}}));
                }
            }
            _ => {}
        }
        self.status(out);
    }

    fn tick(&mut self, out: &mut Outbox, _: f64) {
        let events: Vec<u32> = self.motion.to_vec();
        if self.step() {
            self.controls(out);
        }
        let motion = [self.motion[0] > events[0], self.motion[1] > events[1]];
        if motion.iter().any(|m| *m) {
            self.show_cameras(out, motion);
        }
        if self.ticks & 1 == 0 {
            self.chart(out);
        }
        self.status(out);
    }

    fn tick_interval(&self) -> Option<u32> {
        Some(TICK_MS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hypereffect::server::Loopback;

    fn new() -> Loopback<SmartHome> {
        Loopback::new(Map::new())
    }

    fn ticks(l: &mut Loopback<SmartHome>, n: usize) {
        for i in 0..n {
            l.tick(i as f64);
        }
    }

    fn text_of(l: &Loopback<SmartHome>, id: &str) -> String {
        l.client
            .attr(id, "text")
            .unwrap()
            .as_str()
            .unwrap()
            .to_owned()
    }

    #[test]
    fn controls_change_the_power_draw() {
        let mut l = new();
        assert_eq!(text_of(&l, "b_time"), "Day 1, 18:00");
        assert_eq!(text_of(&l, "draw_0"), "Lights: 28 W");
        assert_eq!(
            l.client.attr("level_kitchen", "disabled"),
            Some(json!(true))
        );
        l.write("light.kitchen.on", json!(true));
        assert!(l.trigger("on_kitchen", "onChange"));
        assert_eq!(
            l.client.attr("level_kitchen", "disabled"),
            Some(json!(false))
        );
        assert_eq!(text_of(&l, "draw_0"), "Lights: 68 W");
        assert!(text_of(&l, "alert_0").ends_with("Kitchen light turned on"));
        l.write("light.kitchen.level", json!(50));
        assert!(l.trigger("level_kitchen", "onChange"));
        assert_eq!(text_of(&l, "draw_0"), "Lights: 48 W");
        assert!(l.client.take_diagnostics().is_empty());
    }

    #[test]
    fn the_thermostat_chases_its_target() {
        let mut l = new();
        l.write("climate.target", json!(23));
        assert!(l.trigger("target", "onChange"));
        assert_eq!(text_of(&l, "b_in"), "Inside 19.5°C");
        assert_eq!(text_of(&l, "t_state"), "Thermostat heating");
        ticks(&mut l, 20);
        let inside: f64 = text_of(&l, "b_in")
            .trim_start_matches("Inside ")
            .trim_end_matches("°C")
            .parse()
            .unwrap();
        assert!((inside - 23.0).abs() < 0.7, "{inside}");
        assert_eq!(text_of(&l, "t_state"), "Thermostat idle");
        // Turned off, the house drifts toward the outdoors, and the target is locked.
        l.write("climate.mode", json!("off"));
        assert!(l.trigger("mode", "onChange"));
        assert_eq!(l.client.attr("target", "disabled"), Some(json!(true)));
        // A window open while it runs gets a warning.
        l.write("climate.mode", json!("heat"));
        l.write("window.kitchen", json!(true));
        assert!(l.trigger("w_kitchen", "onChange"));
        ticks(&mut l, 2);
        assert_eq!(text_of(&l, "b_windows"), "1 window(s) open");
        assert!((0..10).any(|k| text_of(&l, &format!("alert_{k}")).contains("A window is open")));
        assert!(l.client.take_diagnostics().is_empty());
    }

    #[test]
    fn garage_travels_and_locks_buttons() {
        let mut l = new();
        assert_eq!(l.client.attr("garage_close", "disabled"), Some(json!(true)));
        assert!(l.trigger("garage_open", "onPress"));
        assert_eq!(l.client.attr("garage_open", "disabled"), Some(json!(true)));
        assert_eq!(l.client.attr("garage_close", "disabled"), Some(json!(true)));
        ticks(&mut l, 2);
        let half = l
            .client
            .attr("garage_bar", "value")
            .unwrap()
            .as_f64()
            .unwrap();
        assert!(half > 0.0 && half < 1.0, "{half}");
        assert_eq!(
            l.client.attr("garage_bar", "label"),
            Some(json!("Garage opening"))
        );
        ticks(&mut l, 3);
        assert_eq!(
            l.client.attr("garage_bar", "label"),
            Some(json!("Garage open"))
        );
        assert_eq!(
            l.client.attr("garage_close", "disabled"),
            Some(json!(false))
        );
        assert!(l.trigger("garage_close", "onPress"));
        ticks(&mut l, 5);
        assert_eq!(
            l.client.attr("garage_bar", "label"),
            Some(json!("Garage closed"))
        );
    }

    #[test]
    fn scenes_set_everything_at_once() {
        let mut l = new();
        l.write("lock.front", json!(false));
        assert!(l.trigger("lock_front", "onChange"));
        assert_eq!(text_of(&l, "b_doors"), "A door is unlocked");
        assert!(l.trigger("scene_away", "onPress"));
        assert_eq!(l.client.value("lock.front"), Some(&json!(true)));
        assert_eq!(l.client.value("light.living.on"), Some(&json!(false)));
        assert_eq!(l.client.value("cam.front.armed"), Some(&json!(true)));
        assert_eq!(l.client.value("climate.target"), Some(&json!(17.0)));
        assert_eq!(
            l.client.attr("scene_away", "variant"),
            Some(json!("primary"))
        );
        assert_eq!(
            l.client.attr("scene_home", "variant"),
            Some(json!("secondary"))
        );
        assert_eq!(text_of(&l, "b_doors"), "All doors locked");
        assert!(l.trigger("scene_movie", "onPress"));
        assert_eq!(l.client.value("light.living.level"), Some(&json!(15)));
        assert_eq!(
            l.client.value("cam.front.armed"),
            Some(&json!(true)),
            "movie leaves cameras alone"
        );
    }

    #[test]
    fn armed_cameras_see_motion() {
        let mut l = new();
        let first = l.client.attr("shot_front", "src").unwrap();
        assert!(first
            .as_str()
            .unwrap()
            .starts_with("data:image/png;base64,"));
        assert!(l.trigger("scene_away", "onPress"));
        // Nothing is seen by a camera that is off.
        l.write("cam.garden.armed", json!(false));
        assert!(l.trigger("arm_garden", "onChange"));
        ticks(&mut l, 80);
        assert_eq!(text_of(&l, "events_garden"), "0 motion events");
        let seen = text_of(&l, "events_front");
        assert!(!seen.starts_with("0 "), "{seen}");
        assert!((0..10)
            .any(|k| text_of(&l, &format!("alert_{k}")).contains("Motion while nobody is home")));
        assert_ne!(l.client.attr("shot_front", "src").unwrap(), first);
        // Power, charted.
        assert_eq!(l.client.attr("power_chart", "visible"), Some(json!(true)));
        assert!(l.client.take_diagnostics().is_empty());
    }

    #[test]
    fn automations_follow_the_clock() {
        let mut l = new();
        // 18:00 now; at 19:00 the porch light comes on.
        assert_eq!(l.client.value("light.porch.on"), Some(&json!(false)));
        ticks(&mut l, 21);
        assert_eq!(l.client.value("light.porch.on"), Some(&json!(true)));
        // Unlock a door and open the garage; by 23:00 both are put right.
        l.write("lock.back", json!(false));
        assert!(l.trigger("lock_back", "onChange"));
        assert!(l.trigger("garage_open", "onPress"));
        ticks(&mut l, 5);
        assert_eq!(
            l.client.attr("garage_bar", "label"),
            Some(json!("Garage open"))
        );
        ticks(&mut l, 80);
        assert_eq!(l.client.value("lock.back"), Some(&json!(true)));
        assert_eq!(
            l.client.attr("garage_bar", "label"),
            Some(json!("Garage closed"))
        );
        assert!(l.client.take_diagnostics().is_empty());
    }

    #[test]
    fn day_wraps() {
        let mut l = new();
        ticks(&mut l, 130);
        assert!(
            text_of(&l, "b_time").starts_with("Day 2"),
            "{}",
            text_of(&l, "b_time")
        );
        // Crossing a minute of the day, across midnight too.
        let mut h = SmartHome {
            minute: 2,
            ..Default::default()
        };
        assert!(h.crossed(24 * 60 - 1, 0));
        assert!(!h.crossed(24 * 60 - 1, 6 * 60));
        h.minute = 19 * 60;
        assert!(h.crossed(19 * 60 - 3, 19 * 60));
        assert!(!h.crossed(19 * 60, 19 * 60));
    }
}
