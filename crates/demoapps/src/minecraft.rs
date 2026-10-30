//! The Minecraft server administrator demo. No Minecraft is involved: the
//! server is a simulation that lives in the session and runs on the tick.
//!
//! While it runs, players come and go, chat, and load the server, which
//! shows in the TPS, CPU and memory gauges. The admin can start, stop and
//! restart it, kick, ban and op players, edit the settings, run backups
//! and type console commands.

use super::common::{menu, number, replace, text};
use hypereffect::{Action, App, Element, Handler, Outbox, Value, cel, json, ui};
use std::collections::BTreeMap;

const TICK_MS: u32 = 1000;
const LOG_LINES: usize = 14;
const ROSTER: [&str; 12] = [
    "Steve",
    "Alex",
    "Notch_Fan",
    "CreeperSlayer",
    "DiamondDan",
    "Pixel_Pete",
    "RedstoneRita",
    "EnderEli",
    "Zoe_Builds",
    "Moo_Shroom",
    "LavaLarry",
    "Quartz_Quinn",
];
const CHAT: [&str; 8] = [
    "anyone seen my dog?",
    "found diamonds at -312 11 204",
    "creeper behind you!",
    "can someone op me, I swear I'm good",
    "trading 12 iron for a saddle",
    "brb, dinner",
    "who blew up my farm??",
    "gg",
];
const BACKUP_MB: u32 = 412;
/// Ticks the server takes to start and to stop.
const START_TICKS: u32 = 5;
const STOP_TICKS: u32 = 2;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Status {
    Stopped,
    Starting(u32),
    Running,
    Stopping(u32),
}

impl Status {
    fn name(self) -> &'static str {
        match self {
            Status::Stopped => "stopped",
            Status::Starting(_) => "starting",
            Status::Running => "running",
            Status::Stopping(_) => "stopping",
        }
    }
}

struct Player {
    name: String,
    op: bool,
    ping: u32,
}

struct Settings {
    motd: String,
    difficulty: String,
    gamemode: String,
    max_players: usize,
    view_distance: u32,
    pvp: bool,
    whitelist: bool,
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

pub struct Minecraft {
    status: Status,
    restart_after_stop: bool,
    uptime: u64,
    world_ticks: u64,
    weather: &'static str,
    tps: f64,
    cpu: f64,
    mem_mb: f64,
    players: Vec<Player>,
    banned: Vec<String>,
    whitelisted: Vec<String>,
    settings: Settings,
    /// Backup progress, from 0 to 1, while one runs.
    backup: Option<f64>,
    last_backup: String,
    log: Vec<String>,
    rng: Rng,
    /// Ids of the player and ban rows on screen, and what they were drawn from.
    row_ids: Vec<String>,
    drawn: String,
}

impl Default for Minecraft {
    fn default() -> Self {
        Minecraft {
            status: Status::Stopped,
            restart_after_stop: false,
            uptime: 0,
            world_ticks: 1000,
            weather: "clear",
            tps: 0.0,
            cpu: 0.0,
            mem_mb: 0.0,
            players: Vec::new(),
            banned: vec!["xX_Griefer_Xx".into()],
            whitelisted: vec!["Steve".into(), "Alex".into()],
            settings: Settings {
                motd: "A Totally Real Minecraft Server".into(),
                difficulty: "normal".into(),
                gamemode: "survival".into(),
                max_players: 20,
                view_distance: 10,
                pvp: true,
                whitelist: false,
            },
            backup: None,
            last_backup: "never".into(),
            log: Vec::new(),
            rng: Rng(7),
            row_ids: Vec::new(),
            drawn: String::new(),
        }
    }
}

impl Minecraft {
    fn clock(&self) -> String {
        let s = 9 * 3600 + self.uptime;
        format!("{:02}:{:02}:{:02}", s / 3600 % 24, s / 60 % 60, s % 60)
    }

    fn say(&mut self, level: &str, line: impl AsRef<str>) {
        self.log
            .push(format!("[{} {level}]: {}", self.clock(), line.as_ref()));
        if self.log.len() > LOG_LINES {
            self.log.remove(0);
        }
    }

    fn time_of_day(&self) -> String {
        let t = self.world_ticks % 24000;
        let hour = (t / 1000 + 6) % 24;
        let minute = (t % 1000) * 60 / 1000;
        format!(
            "Day {}, {hour:02}:{minute:02}",
            self.world_ticks / 24000 + 1
        )
    }

    fn can_join(&self, name: &str) -> bool {
        !self.players.iter().any(|p| p.name == name)
            && !self.banned.iter().any(|b| b == name)
            && (!self.settings.whitelist || self.whitelisted.iter().any(|w| w == name))
    }

    fn join(&mut self, name: &str) {
        let ping = 18 + self.rng.next(90) as u32;
        self.players.push(Player {
            name: name.into(),
            op: name == "Steve",
            ping,
        });
        self.say("INFO", format!("{name} joined the game"));
    }

    fn leave(&mut self, name: &str, why: Option<&str>) {
        self.players.retain(|p| p.name != name);
        match why {
            Some(w) => self.say("INFO", format!("{name} lost connection: {w}")),
            None => self.say("INFO", format!("{name} left the game")),
        }
    }

    fn begin_stop(&mut self, restart: bool) {
        self.say("INFO", "Stopping the server");
        self.status = Status::Stopping(STOP_TICKS);
        self.restart_after_stop = restart;
        self.backup = None;
        for p in std::mem::take(&mut self.players) {
            self.log
                .push(format!("[{} INFO]: {} left the game", self.clock(), p.name));
        }
        self.log.drain(..self.log.len().saturating_sub(LOG_LINES));
    }

    fn begin_start(&mut self) {
        self.uptime = 0;
        self.log.clear();
        self.say("INFO", "Starting minecraft server version 1.21.4");
        self.say(
            "INFO",
            format!(
                "Loading properties, difficulty: {}",
                self.settings.difficulty
            ),
        );
        self.status = Status::Starting(START_TICKS);
    }

    fn started(&mut self) {
        self.status = Status::Running;
        self.say("INFO", "Done! For help, type \"help\"");
        let first = self.rng.next(ROSTER.len() as u64) as usize;
        for k in 0..3 {
            let name = ROSTER[(first + k * 5) % ROSTER.len()];
            if self.can_join(name) {
                self.join(name);
            }
        }
        self.measure();
    }

    /// The load the server is under, from its players and backup.
    fn measure(&mut self) {
        let n = self.players.len() as f64;
        let backup = if self.backup.is_some() { 1.0 } else { 0.0 };
        let noise = self.rng.next(100) as f64 / 100.0;
        self.tps = (20.0 - 0.12 * n - 2.0 * backup - 0.4 * noise).clamp(10.0, 20.0);
        self.cpu = (0.10 + 0.03 * n + 0.25 * backup + 0.05 * noise).min(1.0);
        let saw = (self.uptime % 120) as f64 / 120.0;
        self.mem_mb = 900.0 + n * 90.0 + saw * 700.0;
    }

    fn step(&mut self) {
        match self.status {
            Status::Stopped => {}
            Status::Starting(left) => {
                self.say(
                    "INFO",
                    format!(
                        "Preparing spawn area: {}%",
                        (START_TICKS - left + 1) * 100 / START_TICKS
                    ),
                );
                if left <= 1 {
                    self.started();
                } else {
                    self.status = Status::Starting(left - 1);
                }
            }
            Status::Stopping(left) => {
                if left <= 1 {
                    self.status = Status::Stopped;
                    self.tps = 0.0;
                    self.cpu = 0.0;
                    self.mem_mb = 0.0;
                    self.say("INFO", "Server stopped");
                    if std::mem::take(&mut self.restart_after_stop) {
                        self.begin_start();
                    }
                } else {
                    self.status = Status::Stopping(left - 1);
                }
            }
            Status::Running => {
                self.uptime += 1;
                self.world_ticks += 20;
                if self.rng.next(100) < 14 && self.players.len() < self.settings.max_players {
                    let name = ROSTER[self.rng.next(ROSTER.len() as u64) as usize];
                    if self.can_join(name) {
                        self.join(name);
                    }
                }
                if self.rng.next(100) < 7 && !self.players.is_empty() {
                    let i = self.rng.next(self.players.len() as u64) as usize;
                    let name = self.players[i].name.clone();
                    self.leave(&name, None);
                }
                if self.rng.next(100) < 10 && !self.players.is_empty() {
                    let i = self.rng.next(self.players.len() as u64) as usize;
                    let line = CHAT[self.rng.next(CHAT.len() as u64) as usize];
                    self.say("INFO", format!("<{}> {line}", self.players[i].name));
                }
                if self.rng.next(200) == 0 {
                    self.weather = if self.weather == "clear" {
                        "rain"
                    } else {
                        "clear"
                    };
                    self.say("INFO", format!("Weather changed to {}", self.weather));
                }
                if let Some(p) = self.backup {
                    if p + 0.1 >= 1.0 {
                        self.backup = None;
                        self.last_backup = self.clock();
                        self.say("INFO", format!("Backup complete ({BACKUP_MB} MB)"));
                    } else {
                        self.backup = Some(p + 0.1);
                    }
                }
                self.measure();
            }
        }
    }

    // --- console -----------------------------------------------------------

    fn command(&mut self, line: &str) {
        self.say("CMD", format!("> {line}"));
        let words: Vec<&str> = line.split_whitespace().collect();
        let Some((cmd, args)) = words.split_first() else {
            return;
        };
        if self.status != Status::Running && !matches!(*cmd, "help" | "start") {
            self.say("WARN", "The server is not running.");
            return;
        }
        let target = args.first().map(|s| s.to_string()).unwrap_or_default();
        match (*cmd, args) {
            ("help", _) => self.say(
                "INFO",
                "list, say, kick, ban, pardon, op, deop, whitelist, time, weather, difficulty, gamemode, tp, give, tps, save-all, stop, restart",
            ),
            ("list", _) => {
                let names: Vec<String> = self.players.iter().map(|p| p.name.clone()).collect();
                self.say("INFO", format!("There are {} of {} players online: {}", names.len(), self.settings.max_players, names.join(", ")));
            }
            ("say", [_, ..]) => self.say("INFO", format!("[Server] {}", args.join(" "))),
            ("tps", _) => self.say("INFO", format!("TPS: {:.1}, uptime {}s", self.tps, self.uptime)),
            ("save-all", _) => self.say("INFO", "Saved the game"),
            ("stop", _) => self.begin_stop(false),
            ("restart", _) => self.begin_stop(true),
            ("kick", [name, reason @ ..]) => {
                if self.players.iter().any(|p| p.name == *name) {
                    let why = if reason.is_empty() { "Kicked by an operator".into() } else { reason.join(" ") };
                    self.leave(name, Some(&why));
                } else {
                    self.say("WARN", format!("No player called {name} is online"));
                }
            }
            ("ban", [name, ..]) => {
                if !self.banned.contains(&target) {
                    self.banned.push(target.clone());
                }
                self.say("INFO", format!("Banned {name}"));
                if self.players.iter().any(|p| p.name == *name) {
                    self.leave(name, Some("Banned by an operator"));
                }
            }
            ("pardon", [name, ..]) => {
                if self.banned.contains(&target) {
                    self.banned.retain(|b| *b != target);
                    self.say("INFO", format!("Unbanned {name}"));
                } else {
                    self.say("WARN", format!("{name} is not banned"));
                }
            }
            ("op" | "deop", [name, ..]) => match self.players.iter_mut().find(|p| p.name == *name) {
                Some(p) => {
                    p.op = *cmd == "op";
                    self.say("INFO", format!("{} {name}", if *cmd == "op" { "Made operator:" } else { "Removed operator:" }));
                }
                None => self.say("WARN", format!("No player called {name} is online")),
            },
            ("whitelist", ["on" | "off", ..]) => {
                self.settings.whitelist = args[0] == "on";
                self.say("INFO", format!("Whitelist is now {}", args[0]));
            }
            ("whitelist", ["add", name, ..]) => {
                if !self.whitelisted.iter().any(|w| w == name) {
                    self.whitelisted.push((*name).to_owned());
                }
                self.say("INFO", format!("Added {name} to the whitelist"));
            }
            ("whitelist", ["remove", name, ..]) => {
                self.whitelisted.retain(|w| w != name);
                self.say("INFO", format!("Removed {name} from the whitelist"));
            }
            ("whitelist", ["list"]) => self.say("INFO", format!("Whitelisted: {}", self.whitelisted.join(", "))),
            ("time", ["set", when]) => {
                let at = match *when {
                    "day" => Some(1000),
                    "noon" => Some(6000),
                    "night" => Some(13000),
                    "midnight" => Some(18000),
                    n => n.parse::<u64>().ok(),
                };
                match at {
                    Some(t) => {
                        self.world_ticks = self.world_ticks / 24000 * 24000 + t % 24000;
                        self.say("INFO", format!("Set the time to {t}"));
                    }
                    None => self.say("WARN", "Usage: time set <day|noon|night|midnight|ticks>"),
                }
            }
            ("weather", [w @ ("clear" | "rain" | "thunder"), ..]) => {
                self.weather = match *w {
                    "rain" => "rain",
                    "thunder" => "thunder",
                    _ => "clear",
                };
                self.say("INFO", format!("Set the weather to {w}"));
            }
            ("difficulty", [d @ ("peaceful" | "easy" | "normal" | "hard"), ..]) => {
                self.settings.difficulty = (*d).to_owned();
                self.say("INFO", format!("The difficulty is now {d}"));
            }
            ("gamemode", [mode @ ("survival" | "creative" | "adventure" | "spectator"), who @ ..]) => {
                match who.first() {
                    Some(name) if !self.players.iter().any(|p| p.name == *name) => {
                        self.say("WARN", format!("No player called {name} is online"))
                    }
                    Some(name) => self.say("INFO", format!("Set {name}'s game mode to {mode}")),
                    None => {
                        self.settings.gamemode = (*mode).to_owned();
                        self.say("INFO", format!("The default game mode is now {mode}"));
                    }
                }
            }
            ("tp", [a, b]) => {
                let online = |n: &str| self.players.iter().any(|p| p.name == n);
                if online(a) && online(b) {
                    self.say("INFO", format!("Teleported {a} to {b}"));
                } else {
                    self.say("WARN", "Both players must be online");
                }
            }
            ("give", [name, item, rest @ ..]) => {
                let n = rest.first().and_then(|n| n.parse::<u32>().ok()).unwrap_or(1);
                if self.players.iter().any(|p| p.name == *name) {
                    self.say("INFO", format!("Gave {n} [{item}] to {name}"));
                } else {
                    self.say("WARN", format!("No player called {name} is online"));
                }
            }
            _ => self.say("WARN", "Unknown or incomplete command, see \"help\""),
        }
    }

    // --- display -----------------------------------------------------------

    fn show(&mut self, out: &mut Outbox) {
        let (status, label) = match self.status {
            Status::Stopped => ("stopped", "Offline".to_owned()),
            Status::Starting(l) => (
                "starting",
                format!("Starting… {}%", (START_TICKS - l) * 100 / START_TICKS),
            ),
            Status::Running => ("running", "Online".to_owned()),
            Status::Stopping(_) => ("stopping", "Stopping…".to_owned()),
        };
        let up = if self.status == Status::Running {
            format!(
                "{}h {:02}m {:02}s",
                self.uptime / 3600,
                self.uptime / 60 % 60,
                self.uptime % 60
            )
        } else {
            "—".to_owned()
        };
        let mut lines: Vec<String> = self.log.clone();
        lines.resize(LOG_LINES, String::new());
        let (tps_kind, tps_text) = match self.status {
            Status::Running => (
                if self.tps >= 18.0 {
                    "success"
                } else if self.tps >= 15.0 {
                    "warning"
                } else {
                    "failure"
                },
                format!("TPS {:.1}", self.tps),
            ),
            _ => ("neutral", "TPS —".to_owned()),
        };
        out.signal(json!({
            "srv": {
                "status": status,
                "label": label,
                "players": format!("{} / {} players", self.players.len(), self.settings.max_players),
                "uptime": format!("Uptime {up}"),
                "tps": tps_text,
                "tps_kind": tps_kind,
                "time": self.time_of_day(),
                "weather": self.weather,
                "motd": self.settings.motd,
            },
            "cpu": self.cpu,
            "cpu_text": format!("CPU {:.0}%", self.cpu * 100.0),
            "mem": self.mem_mb / 4096.0,
            "mem_text": format!("Memory {:.0} / 4096 MB", self.mem_mb),
            "backup": {
                "running": self.backup.is_some(),
                "progress": self.backup.unwrap_or(0.0),
                "text": match self.backup {
                    Some(p) => format!("Backing up… {:.0}%", p * 100.0),
                    None => format!("Last backup: {}", self.last_backup),
                },
            },
            "log": lines,
            "online": self.players.iter().map(|p| p.name.clone()).collect::<Vec<_>>(),
        }));
        // The player and ban rows are redrawn only when what they show changes.
        let sig = format!(
            "{:?}|{:?}|{}",
            self.players
                .iter()
                .map(|p| (&p.name, p.op, p.ping))
                .collect::<Vec<_>>(),
            self.banned,
            self.status.name()
        );
        if sig != self.drawn {
            self.drawn = sig;
            let mut rows: Vec<Element> = Vec::new();
            for p in &self.players {
                let n = &p.name;
                rows.push(
                    ui::row(format!("row_{n}"))
                        .parent("online_list")
                        .gap("small")
                        .wrap(true),
                );
                rows.push(
                    ui::text(format!("who_{n}"))
                        .parent(format!("row_{n}"))
                        .text(format!(
                            "{n}{} — {} ms",
                            if p.op { " (op)" } else { "" },
                            p.ping
                        )),
                );
                rows.push(
                    ui::button(format!("op_{n}"))
                        .parent(format!("row_{n}"))
                        .label(if p.op { "Deop" } else { "Op" })
                        .on_press(Handler::action("op").payload(n.as_str())),
                );
                rows.push(
                    ui::button(format!("kick_{n}"))
                        .parent(format!("row_{n}"))
                        .label("Kick")
                        .on_press(Handler::action("kick").payload(n.as_str())),
                );
                rows.push(
                    ui::button(format!("ban_{n}"))
                        .parent(format!("row_{n}"))
                        .label("Ban")
                        .variant("failure")
                        .on_press(Handler::action("ban").payload(n.as_str())),
                );
            }
            for b in &self.banned {
                rows.push(
                    ui::row(format!("brow_{b}"))
                        .parent("banned_list")
                        .gap("small"),
                );
                rows.push(
                    ui::text(format!("bwho_{b}"))
                        .parent(format!("brow_{b}"))
                        .text(b.as_str()),
                );
                rows.push(
                    ui::button(format!("pardon_{b}"))
                        .parent(format!("brow_{b}"))
                        .label("Pardon")
                        .on_press(Handler::action("pardon").payload(b.as_str())),
                );
            }
            replace(out, &mut self.row_ids, rows);
        }
    }

    fn settings_signal(&self) -> Value {
        let s = &self.settings;
        json!({"cfg": {
            "motd": s.motd, "difficulty": s.difficulty, "gamemode": s.gamemode,
            "max_players": s.max_players, "view": s.view_distance, "pvp": s.pvp, "whitelist": s.whitelist,
        }})
    }
}

impl App for Minecraft {
    fn hello(&mut self, out: &mut Outbox, _: &BTreeMap<String, String>) {
        out.signal(json!({
            "menu": {
                "difficulty": menu(&[("peaceful", "Peaceful"), ("easy", "Easy"), ("normal", "Normal"), ("hard", "Hard")]),
                "gamemode": menu(&[("survival", "Survival"), ("creative", "Creative"), ("adventure", "Adventure"), ("spectator", "Spectator")]),
            },
            "form": {"command": "", "whitelist": ""},
            "feedback": {"kind": "info", "text": ""},
        }));
        out.signal(self.settings_signal());
        out.element(ui::column("main").gap("large"));
        out.element(
            ui::text("title")
                .parent("main")
                .variant("h1")
                .text("Blockhouse — Server Admin"),
        );
        out.element(ui::row("header").parent("main").gap("medium").wrap(true));
        out.element(
            ui::badge("status")
                .parent("header")
                .variant(cel("s.srv.status == 'running' ? 'success' : (s.srv.status == 'stopped' ? 'failure' : 'warning')"))
                .text(cel("s.srv.label")),
        );
        out.element(
            ui::badge("players_badge")
                .parent("header")
                .variant("info")
                .text(cel("s.srv.players")),
        );
        out.element(
            ui::badge("tps_badge")
                .parent("header")
                .variant(cel("s.srv.tps_kind"))
                .text(cel("s.srv.tps")),
        );
        out.element(ui::row("power").parent("main").gap("small").wrap(true));
        out.element(
            ui::button("start")
                .parent("power")
                .label("Start")
                .variant("success")
                .disabled(cel("s.srv.status != 'stopped'"))
                .on_press(Handler::action("start")),
        );
        out.element(
            ui::button("stop")
                .parent("power")
                .label("Stop")
                .variant("failure")
                .disabled(cel("s.srv.status != 'running'"))
                .on_press(Handler::action("stop")),
        );
        out.element(
            ui::button("restart")
                .parent("power")
                .label("Restart")
                .disabled(cel("s.srv.status != 'running'"))
                .on_press(Handler::action("restart")),
        );
        out.element(
            ui::notice("feedback")
                .parent("main")
                .variant(cel("s.feedback.kind"))
                .visible(cel("s.feedback.text != ''"))
                .text(cel("s.feedback.text")),
        );
        out.element(ui::tabs("tabs").parent("main"));

        // Overview.
        out.element(ui::tab("overview").parent("tabs").label("Overview"));
        out.element(
            ui::text("motd")
                .parent("overview")
                .variant("h3")
                .text(cel("s.srv.motd")),
        );
        out.element(ui::row("facts").parent("overview").gap("small").wrap(true));
        out.element(
            ui::badge("uptime")
                .parent("facts")
                .text(cel("s.srv.uptime")),
        );
        out.element(
            ui::badge("world_time")
                .parent("facts")
                .text(cel("s.srv.time")),
        );
        out.element(
            ui::badge("weather")
                .parent("facts")
                .text(cel("'Weather: ' + s.srv.weather")),
        );
        out.element(
            ui::progress("cpu")
                .parent("overview")
                .value(cel("s.cpu"))
                .label(cel("s.cpu_text")),
        );
        out.element(
            ui::progress("mem")
                .parent("overview")
                .value(cel("s.mem"))
                .label(cel("s.mem_text")),
        );
        out.element(
            ui::progress("backup_bar")
                .parent("overview")
                .value(cel("s.backup.progress"))
                .label(cel("s.backup.text")),
        );
        out.element(
            ui::button("backup")
                .parent("overview")
                .label("Back up the world now")
                .disabled(cel("s.srv.status != 'running' || s.backup.running"))
                .on_press(Handler::action("backup")),
        );

        // Players.
        out.element(ui::tab("players").parent("tabs").label("Players"));
        out.element(
            ui::text("online_title")
                .parent("players")
                .variant("h3")
                .text("Online"),
        );
        out.element(
            ui::list("online_list")
                .parent("players")
                .empty("Nobody is online."),
        );
        out.element(
            ui::text("banned_title")
                .parent("players")
                .variant("h3")
                .text("Banned"),
        );
        out.element(
            ui::list("banned_list")
                .parent("players")
                .empty("Nobody is banned."),
        );
        out.element(
            ui::input("whitelist_name")
                .parent("players")
                .label("Add to the whitelist")
                .bind("form.whitelist")
                .placeholder("Player name")
                .on_submit(Handler::action("whitelist")),
        );
        out.element(
            ui::button("whitelist_add")
                .parent("players")
                .label("Add")
                .on_press(Handler::action("whitelist")),
        );

        // Console.
        out.element(ui::tab("console").parent("tabs").label("Console"));
        for k in 0..LOG_LINES {
            out.element(
                ui::text(format!("log_{k}"))
                    .parent("console")
                    .variant("code")
                    .visible(cel(format!("s.log[{k}] != ''")))
                    .text(cel(format!("s.log[{k}]"))),
            );
        }
        out.element(
            ui::input("command")
                .parent("console")
                .label("Command")
                .bind("form.command")
                .placeholder("help")
                .on_submit(Handler::action("command")),
        );
        out.element(
            ui::button("send")
                .parent("console")
                .label("Send")
                .variant("primary")
                .on_press(Handler::action("command")),
        );

        // Settings.
        out.element(ui::tab("settings").parent("tabs").label("Settings"));
        out.element(
            ui::input("cfg_motd")
                .parent("settings")
                .label("Message of the day")
                .bind("cfg.motd"),
        );
        out.element(
            ui::select("cfg_difficulty")
                .parent("settings")
                .label("Difficulty")
                .bind("cfg.difficulty")
                .options(cel("s.menu.difficulty")),
        );
        out.element(
            ui::select("cfg_gamemode")
                .parent("settings")
                .label("Default game mode")
                .bind("cfg.gamemode")
                .options(cel("s.menu.gamemode")),
        );
        out.element(
            ui::slider("cfg_max")
                .parent("settings")
                .label("Max players")
                .min(1)
                .max(100)
                .bind("cfg.max_players"),
        );
        out.element(
            ui::slider("cfg_view")
                .parent("settings")
                .label("View distance (chunks)")
                .min(2)
                .max(32)
                .bind("cfg.view"),
        );
        out.element(
            ui::checkbox("cfg_pvp")
                .parent("settings")
                .label("PvP")
                .variant("switch")
                .bind("cfg.pvp"),
        );
        out.element(
            ui::checkbox("cfg_whitelist")
                .parent("settings")
                .label("Whitelist only")
                .variant("switch")
                .bind("cfg.whitelist"),
        );
        out.element(
            ui::button("apply")
                .parent("settings")
                .label("Save settings")
                .variant("primary")
                .on_press(Handler::action("apply")),
        );
        // After the elements: it draws rows into some of them.
        self.show(out);
    }

    fn action(&mut self, out: &mut Outbox, action: &Action) {
        let feedback = |kind: &str, t: &str| json!({"feedback": {"kind": kind, "text": t}});
        out.signal(feedback("info", ""));
        let name: String = action.payload_as().unwrap_or_default();
        match action.name.as_str() {
            "start" if self.status == Status::Stopped => self.begin_start(),
            "stop" if self.status == Status::Running => self.begin_stop(false),
            "restart" if self.status == Status::Running => self.begin_stop(true),
            "kick" => self.command(&format!("kick {name}")),
            "ban" => self.command(&format!("ban {name}")),
            "op" => {
                let op = self
                    .players
                    .iter()
                    .find(|p| p.name == name)
                    .is_some_and(|p| p.op);
                self.command(&format!("{} {name}", if op { "deop" } else { "op" }));
            }
            "pardon" => self.command(&format!("pardon {name}")),
            "whitelist" => {
                let who = text(action, "form.whitelist");
                if who.is_empty() || !who.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                    out.signal(feedback(
                        "failure",
                        "Enter a player name: letters, digits and underscores.",
                    ));
                } else {
                    self.whitelisted.push(who.clone());
                    self.say("INFO", format!("Added {who} to the whitelist"));
                    out.signal(json!({"form": {"whitelist": ""}}));
                }
            }
            "backup" if self.status == Status::Running && self.backup.is_none() => {
                self.say("INFO", "Starting a backup of the world");
                self.backup = Some(0.0);
            }
            "command" => {
                let line = text(action, "form.command");
                if !line.is_empty() {
                    self.command(&line);
                }
                out.signal(json!({"form": {"command": ""}}));
            }
            "apply" => {
                let motd = text(action, "cfg.motd");
                if motd.is_empty() {
                    out.signal(feedback(
                        "failure",
                        "The message of the day cannot be empty.",
                    ));
                } else {
                    let s = &mut self.settings;
                    s.motd = motd.chars().take(60).collect();
                    s.difficulty = text(action, "cfg.difficulty");
                    s.gamemode = text(action, "cfg.gamemode");
                    s.max_players = number(action, "cfg.max_players")
                        .unwrap_or(20.0)
                        .clamp(1.0, 100.0) as usize;
                    let view = number(action, "cfg.view").unwrap_or(10.0).clamp(2.0, 32.0) as u32;
                    let changed_view = view != s.view_distance;
                    s.view_distance = view;
                    s.pvp = action
                        .signal("cfg.pvp")
                        .and_then(Value::as_bool)
                        .unwrap_or(true);
                    s.whitelist = action
                        .signal("cfg.whitelist")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    self.say("INFO", "Settings saved");
                    out.signal(feedback(
                        "success",
                        if changed_view && self.status == Status::Running {
                            "Saved. The view distance applies after a restart."
                        } else {
                            "Settings saved."
                        },
                    ));
                }
            }
            _ => {}
        }
        out.signal(self.settings_signal());
        self.show(out);
    }

    fn tick(&mut self, out: &mut Outbox, _: f64) {
        self.step();
        self.show(out);
    }

    fn tick_interval(&self) -> Option<u32> {
        (self.status != Status::Stopped).then_some(TICK_MS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hypereffect::server::Loopback;

    fn new() -> Loopback<Minecraft> {
        Loopback::new()
    }

    fn run_command(l: &mut Loopback<Minecraft>, cmd: &str) {
        l.write("form.command", json!(cmd));
        assert!(l.trigger("send", "onPress"));
    }

    fn last_log(l: &Loopback<Minecraft>) -> String {
        let lines = l.client.value("log").unwrap().as_array().unwrap().clone();
        lines
            .iter()
            .rev()
            .find_map(|v| v.as_str().filter(|s| !s.is_empty()))
            .unwrap()
            .to_owned()
    }

    fn boot(l: &mut Loopback<Minecraft>) {
        assert!(l.trigger("start", "onPress"));
        for t in 0..START_TICKS {
            l.tick(t as f64);
        }
    }

    #[test]
    fn starts_runs_and_stops() {
        let mut l = new();
        assert_eq!(l.client.attr("status", "text"), Some(json!("Offline")));
        assert_eq!(l.client.attr("stop", "disabled"), Some(json!(true)));
        assert_eq!(l.server.tick_interval(), None);
        assert!(l.trigger("start", "onPress"));
        assert_eq!(l.client.attr("start", "disabled"), Some(json!(true)));
        assert_eq!(l.client.attr("status", "text"), Some(json!("Starting… 0%")));
        assert_eq!(l.server.tick_interval(), Some(TICK_MS));
        for t in 0..START_TICKS {
            l.tick(t as f64);
        }
        assert_eq!(l.client.attr("status", "text"), Some(json!("Online")));
        assert_eq!(l.client.attr("stop", "disabled"), Some(json!(false)));
        assert_eq!(l.client.attr("backup", "disabled"), Some(json!(false)));
        let online = l.client.value("online").unwrap().as_array().unwrap().len();
        assert!(online >= 1);
        assert!(l.client.attr("cpu", "value").unwrap().as_f64().unwrap() > 0.0);
        // Let it run a while: it stays healthy and the clock moves.
        for t in 0..60 {
            l.tick(t as f64);
        }
        assert!(
            l.client
                .attr("uptime", "text")
                .unwrap()
                .as_str()
                .unwrap()
                .contains("1m")
        );
        let tps = l.client.attr("tps_badge", "text").unwrap();
        assert!(tps.as_str().unwrap().starts_with("TPS "));
        // Stop.
        assert!(l.trigger("stop", "onPress"));
        for t in 0..STOP_TICKS {
            l.tick(t as f64);
        }
        assert_eq!(l.client.attr("status", "text"), Some(json!("Offline")));
        assert_eq!(l.client.value("online"), Some(&json!([])));
        assert_eq!(l.server.tick_interval(), None);
        let d = l.client.take_diagnostics();
        assert!(d.is_empty(), "{d:?}");
    }

    #[test]
    fn restart_comes_back() {
        let mut l = new();
        boot(&mut l);
        assert!(l.trigger("restart", "onPress"));
        for t in 0..(STOP_TICKS + START_TICKS) {
            l.tick(t as f64);
        }
        assert_eq!(l.client.attr("status", "text"), Some(json!("Online")));
    }

    #[test]
    fn player_actions() {
        let mut l = new();
        boot(&mut l);
        let name = l.client.value("online").unwrap()[0]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(l.client.contains(&format!("kick_{name}")));
        // Kick.
        assert!(l.trigger(&format!("kick_{name}"), "onPress"));
        assert!(
            !l.client
                .value("online")
                .unwrap()
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v == &json!(name))
        );
        assert!(!l.client.contains(&format!("kick_{name}")));
        // Ban another; the ban shows and can be lifted.
        let other = l.client.value("online").unwrap()[0]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(l.trigger(&format!("ban_{other}"), "onPress"));
        assert!(l.client.contains(&format!("pardon_{other}")));
        assert!(l.trigger(&format!("pardon_{other}"), "onPress"));
        assert!(!l.client.contains(&format!("pardon_{other}")));
        assert!(l.client.contains("pardon_xX_Griefer_Xx"));
        // Op toggles.
        let who = l.client.value("online").unwrap()[0]
            .as_str()
            .unwrap()
            .to_owned();
        let before = l.client.attr(&format!("op_{who}"), "label").unwrap();
        assert!(l.trigger(&format!("op_{who}"), "onPress"));
        assert_ne!(
            l.client.attr(&format!("op_{who}"), "label").unwrap(),
            before
        );
        let d = l.client.take_diagnostics();
        assert!(d.is_empty(), "{d:?}");
    }

    #[test]
    fn console_commands() {
        let mut l = new();
        run_command(&mut l, "list");
        assert!(last_log(&l).contains("not running"));
        boot(&mut l);
        run_command(&mut l, "say hello world");
        assert!(last_log(&l).ends_with("[Server] hello world"));
        assert_eq!(l.client.value("form.command"), Some(&json!("")));
        run_command(&mut l, "time set night");
        assert_eq!(
            l.client.attr("world_time", "text"),
            Some(json!("Day 1, 19:00"))
        );
        run_command(&mut l, "weather thunder");
        assert_eq!(
            l.client.attr("weather", "text"),
            Some(json!("Weather: thunder"))
        );
        run_command(&mut l, "difficulty hard");
        assert_eq!(l.client.value("cfg.difficulty"), Some(&json!("hard")));
        run_command(&mut l, "whitelist add Newbie");
        run_command(&mut l, "whitelist list");
        assert!(last_log(&l).contains("Newbie"));
        run_command(&mut l, "kick Nobody");
        assert!(last_log(&l).contains("No player called Nobody"));
        run_command(&mut l, "frobnicate");
        assert!(last_log(&l).contains("Unknown"));
        run_command(&mut l, "stop");
        assert_eq!(l.client.attr("status", "text"), Some(json!("Stopping…")));
        let d = l.client.take_diagnostics();
        assert!(d.is_empty(), "{d:?}");
    }

    #[test]
    fn backup_and_settings() {
        let mut l = new();
        boot(&mut l);
        assert!(l.trigger("backup", "onPress"));
        assert_eq!(l.client.attr("backup", "disabled"), Some(json!(true)));
        for t in 0..5 {
            l.tick(t as f64);
        }
        let p = l
            .client
            .attr("backup_bar", "value")
            .unwrap()
            .as_f64()
            .unwrap();
        assert!(p > 0.3 && p < 1.0, "{p}");
        for t in 0..10 {
            l.tick(t as f64);
        }
        assert_eq!(l.client.attr("backup", "disabled"), Some(json!(false)));
        assert!(
            l.client
                .attr("backup_bar", "label")
                .unwrap()
                .as_str()
                .unwrap()
                .starts_with("Last backup: 09:")
        );
        // Settings.
        l.write("cfg.motd", json!("Hello there"));
        l.write("cfg.max_players", json!(5));
        l.write("cfg.view", json!(16));
        l.write("cfg.whitelist", json!(true));
        assert!(l.trigger("apply", "onPress"));
        assert_eq!(l.client.attr("motd", "text"), Some(json!("Hello there")));
        assert_eq!(
            l.client
                .attr("players_badge", "text")
                .unwrap()
                .as_str()
                .unwrap()
                .split(" / ")
                .nth(1),
            Some("5 players")
        );
        assert_eq!(
            l.client.attr("feedback", "text"),
            Some(json!("Saved. The view distance applies after a restart."))
        );
        l.write("cfg.motd", json!(""));
        assert!(l.trigger("apply", "onPress"));
        assert_eq!(l.client.attr("feedback", "variant"), Some(json!("failure")));
        // The whitelist form.
        l.write("form.whitelist", json!("bad name!"));
        assert!(l.trigger("whitelist_add", "onPress"));
        assert_eq!(l.client.attr("feedback", "variant"), Some(json!("failure")));
        let d = l.client.take_diagnostics();
        assert!(d.is_empty(), "{d:?}");
    }
}
