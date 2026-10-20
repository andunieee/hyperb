//! The financial calculator demo: an RPN machine in the spirit of the HP-12C.
//!
//! Numbers go on a four-level stack (X, Y, Z, T): type a number, press ENTER
//! to push it, and operators take their operands from the stack, so
//! `2 ENTER 3 + 4 ×` is (2+3)×4. The time-value-of-money keys (n, i, PV,
//! PMT, FV) store the number being typed, or solve for their register when
//! pressed without one. There are also cash flows with NPV and IRR,
//! statistics, percent keys and ten storage registers.
//!
//! Every key is a button that sends its name as an action; the machine lives
//! on the server and shows itself through signals.

use super::common::group;
use hypereffect::{cel, json, ui, Action, App, Handler, Map, Outbox, Value};

/// Keys in display order, by row: code, label, variant.
const KEY_ROWS: &[&[(&str, &str, &str)]] = &[
    &[
        ("n", "n", "success"),
        ("i", "i", "success"),
        ("pv", "PV", "success"),
        ("pmt", "PMT", "success"),
        ("fv", "FV", "success"),
        ("12x", "12×", "secondary"),
        ("12/", "12÷", "secondary"),
    ],
    &[
        ("beg", "BEGIN", "secondary"),
        ("end", "END", "secondary"),
        ("cf0", "CF0", "secondary"),
        ("cfj", "CFj", "secondary"),
        ("nj", "Nj", "secondary"),
        ("npv", "NPV", "success"),
        ("irr", "IRR", "success"),
    ],
    &[
        ("pct", "%", "secondary"),
        ("dpct", "Δ%", "secondary"),
        ("pctt", "%T", "secondary"),
        ("sum+", "Σ+", "secondary"),
        ("sum-", "Σ−", "secondary"),
        ("mean", "x̄", "secondary"),
        ("sdev", "s", "secondary"),
        ("yhat", "ŷ,r", "secondary"),
    ],
    &[
        ("pow", "yˣ", "secondary"),
        ("inv", "1/x", "secondary"),
        ("sqrt", "√x", "secondary"),
        ("sq", "x²", "secondary"),
        ("exp", "eˣ", "secondary"),
        ("ln", "LN", "secondary"),
        ("pi", "π", "secondary"),
    ],
    &[
        ("chs", "CHS", "secondary"),
        ("swap", "x≷y", "secondary"),
        ("roll", "R↓", "secondary"),
        ("sto", "STO", "secondary"),
        ("rcl", "RCL", "secondary"),
        ("bksp", "⌫", "plain"),
        ("clx", "CLx", "failure"),
    ],
    &[
        ("clfin", "CLEAR FIN", "failure"),
        ("clreg", "CLEAR REG", "failure"),
    ],
    &[
        ("7", "7", "plain"),
        ("8", "8", "plain"),
        ("9", "9", "plain"),
        ("div", "÷", "primary"),
    ],
    &[
        ("4", "4", "plain"),
        ("5", "5", "plain"),
        ("6", "6", "plain"),
        ("mul", "×", "primary"),
    ],
    &[
        ("1", "1", "plain"),
        ("2", "2", "plain"),
        ("3", "3", "plain"),
        ("sub", "−", "primary"),
    ],
    &[
        ("0", "0", "plain"),
        ("dot", ".", "plain"),
        ("enter", "ENTER", "primary"),
        ("add", "+", "primary"),
    ],
];

/// Registers for n, i, PV, PMT and FV.
const N: usize = 0;
const I: usize = 1;
const PV: usize = 2;
const PMT: usize = 3;
const FV: usize = 4;
const TVM_NAMES: [&str; 5] = ["n", "i", "pv", "pmt", "fv"];

/// The most digits a typed number takes.
const MAX_DIGITS: usize = 15;

#[derive(Clone, Copy, PartialEq, Default)]
enum Prefix {
    #[default]
    None,
    Sto,
    Rcl,
}

#[derive(Default)]
struct Stats {
    n: f64,
    sx: f64,
    sxx: f64,
    sy: f64,
    syy: f64,
    sxy: f64,
}

#[derive(Default)]
pub struct Finance {
    /// X, Y, Z, T.
    stack: [f64; 4],
    /// The digits being typed into X, if a number is being typed.
    entry: Option<String>,
    /// Whether typing a new number first pushes the stack up.
    lift: bool,
    /// Whether X holds a number the user typed, entered or recalled, which a
    /// TVM key stores instead of solving.
    storable: bool,
    prefix: Prefix,
    regs: [f64; 10],
    /// n, i (percent), PV, PMT, FV.
    tvm: [f64; 5],
    begin: bool,
    stats: Stats,
    cf0: f64,
    flows: Vec<(f64, u32)>,
    decimals: usize,
    error: String,
}

// --- arithmetic --------------------------------------------------------------

/// The value of one payment per period, as in
/// `PV + PMT·annuity + FV·(1+i)⁻ⁿ = 0`, for a rate `i` as a fraction.
fn annuity(n: f64, i: f64, begin: bool) -> f64 {
    let due = if begin { 1.0 + i } else { 1.0 };
    if i.abs() < 1e-12 {
        n
    } else {
        due * (1.0 - (1.0 + i).powf(-n)) / i
    }
}

/// Finds a root of `f` over rates from -100% to 10000%, starting from 0 and
/// widening, so the smallest rate wins.
fn solve(f: impl Fn(f64) -> f64) -> Option<f64> {
    let bisect = |mut lo: f64, mut hi: f64| {
        let flo = f(lo);
        for _ in 0..200 {
            let mid = (lo + hi) / 2.0;
            let fm = f(mid);
            if (fm < 0.0) == (flo < 0.0) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        (lo + hi) / 2.0
    };
    if f(0.0) == 0.0 {
        return Some(0.0);
    }
    for sign in [1.0, -1.0] {
        let mut lo = 0.0;
        let mut j = 0;
        loop {
            let hi = sign * 1e-5 * 1.05f64.powi(j);
            if hi.abs() > 100.0 || hi <= -0.9999 {
                break;
            }
            let (flo, fhi) = (f(lo), f(hi));
            if flo.is_finite() && fhi.is_finite() && (flo < 0.0) != (fhi < 0.0) {
                return Some(bisect(lo, hi));
            }
            lo = hi;
            j += 1;
        }
    }
    None
}

impl Finance {
    fn tvm_equation(&self, i: f64) -> f64 {
        let [n, _, pv, pmt, fv] = self.tvm;
        pv + pmt * annuity(n, i, self.begin) + fv * (1.0 + i).powf(-n)
    }

    /// Solves the TVM register `which` from the other four.
    fn solve_tvm(&self, which: usize) -> Result<f64, &'static str> {
        let [n, rate, pv, pmt, fv] = self.tvm;
        let i = rate / 100.0;
        let v = (1.0 + i).powf(-n);
        let a = annuity(n, i, self.begin);
        let r = match which {
            PV => -(pmt * a + fv * v),
            PMT if a.abs() > 1e-12 => -(pv + fv * v) / a,
            PMT => return Err("Error: n is zero"),
            FV => -(pv + pmt * a) / v,
            N if i.abs() < 1e-12 => {
                if pmt == 0.0 {
                    return Err("Error: PMT is zero");
                }
                -(pv + fv) / pmt
            }
            N => {
                let due = if self.begin { 1.0 + i } else { 1.0 };
                let c = pmt * due / i;
                let ratio = (c - fv) / (c + pv);
                if !(ratio.is_finite() && ratio > 0.0) {
                    return Err("Error: no solution for n");
                }
                ratio.ln() / (1.0 + i).ln()
            }
            _ => {
                return solve(|i| self.tvm_equation(i))
                    .map(|i| i * 100.0)
                    .ok_or("Error: no solution for i")
            }
        };
        if r.is_finite() {
            Ok(r)
        } else {
            Err("Error: cannot be solved")
        }
    }

    /// The net present value of the cash flows at `rate` (a fraction).
    fn npv_at(&self, rate: f64) -> f64 {
        let mut total = self.cf0;
        let mut k = 0;
        for (amount, count) in &self.flows {
            for _ in 0..*count {
                k += 1;
                total += amount / (1.0 + rate).powi(k);
            }
        }
        total
    }
}

/// A number with `d` decimals and thousands separators.
fn fixed(v: f64, d: usize) -> String {
    if !v.is_finite() {
        return "Error".into();
    }
    if v.abs() >= 1e15 {
        return format!("{v:.*e}", d.min(6));
    }
    let s = format!("{:.*}", d, v.abs());
    let (int, frac) = s.split_once('.').unwrap_or((&s, ""));
    let neg = v < 0.0 && s.chars().any(|c| c.is_ascii_digit() && c != '0');
    let mut out = format!("{}{}", if neg { "-" } else { "" }, group(int));
    if !frac.is_empty() {
        out.push('.');
        out.push_str(frac);
    }
    out
}

/// A register's value, with as many decimals as it needs, up to six.
fn exact(v: f64) -> String {
    let s = fixed(v, 6);
    if s.contains('.') && !s.contains('e') {
        s.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        s
    }
}

// --- the keys ----------------------------------------------------------------

impl Finance {
    fn x(&self) -> f64 {
        self.stack[0]
    }

    fn commit(&mut self) {
        if let Some(e) = self.entry.take() {
            self.stack[0] = e.parse().unwrap_or(0.0);
        }
    }

    fn push_up(&mut self) {
        self.stack = [self.stack[0], self.stack[0], self.stack[1], self.stack[2]];
    }

    /// Puts a number in X, lifting the stack if it is enabled.
    fn put(&mut self, v: f64) {
        if self.lift {
            self.push_up();
        }
        self.stack[0] = v;
        self.lift = true;
    }

    /// Replaces X with a result.
    fn result(&mut self, v: f64) {
        if v.is_finite() {
            self.stack[0] = v;
            self.lift = true;
        } else {
            self.error = "Error: out of range".into();
        }
    }

    /// Replaces X and Y with the result of an operator, dropping the stack.
    fn binary(&mut self, f: impl Fn(f64, f64) -> Option<f64>) {
        let [x, y, z, t] = self.stack;
        match f(y, x).filter(|r| r.is_finite()) {
            Some(r) => {
                self.stack = [r, z, t, t];
                self.lift = true;
            }
            None => self.error = "Error: invalid operation".into(),
        }
    }

    fn unary(&mut self, f: impl Fn(f64) -> Option<f64>) {
        match f(self.x()).filter(|r| r.is_finite()) {
            Some(r) => self.result(r),
            None => self.error = "Error: invalid operation".into(),
        }
    }

    /// Pushes two results: `a` in X and `b` in Y.
    fn pair(&mut self, a: f64, b: f64) {
        let [x, y, ..] = self.stack;
        self.stack = [a, b, x, y];
        self.lift = true;
    }

    fn type_char(&mut self, key: &str) {
        if self.entry.is_none() {
            if self.lift {
                self.push_up();
            }
            self.stack[0] = 0.0;
            self.entry = Some(String::new());
        }
        let e = self.entry.as_mut().expect("entry started");
        if key == "dot" {
            if !e.contains('.') {
                if e.is_empty() || e == "-" {
                    e.push('0');
                }
                e.push('.');
            }
        } else {
            if e == "0" {
                e.clear();
            }
            if e.chars().filter(char::is_ascii_digit).count() < MAX_DIGITS {
                e.push_str(key);
            }
        }
        self.storable = true;
    }

    fn change_sign(&mut self) {
        match &mut self.entry {
            Some(e) if e.starts_with('-') => {
                e.remove(0);
            }
            Some(e) => e.insert(0, '-'),
            None => self.stack[0] = -self.stack[0],
        }
    }

    fn backspace(&mut self) {
        match &mut self.entry {
            Some(e) => {
                e.pop();
            }
            None => {
                self.stack[0] = 0.0;
                self.lift = false;
                self.storable = false;
            }
        }
    }

    fn memory(&mut self, prefix: Prefix, digit: usize) {
        match prefix {
            Prefix::Sto => {
                self.commit();
                self.regs[digit] = self.x();
                self.lift = true;
            }
            _ => {
                self.put(self.regs[digit]);
                self.storable = true;
            }
        }
    }

    fn tvm_key(&mut self, which: usize, typed: bool) {
        if typed {
            self.tvm[which] = self.x();
            self.lift = true;
            return;
        }
        match self.solve_tvm(which) {
            Ok(r) => {
                self.tvm[which] = r;
                self.result(r);
            }
            Err(e) => self.error = e.into(),
        }
    }

    fn statistic(&mut self, key: &str) {
        let s = &self.stats;
        let n = s.n;
        if n < 1.0 || (n < 2.0 && key != "mean") {
            self.error = "Error: not enough data".into();
            return;
        }
        let var = |sxx: f64, sx: f64| ((sxx - sx * sx / n) / (n - 1.0)).max(0.0);
        match key {
            "mean" => self.pair(s.sx / n, s.sy / n),
            "sdev" => self.pair(var(s.sxx, s.sx).sqrt(), var(s.syy, s.sy).sqrt()),
            _ => {
                let dx = n * s.sxx - s.sx * s.sx;
                let dy = n * s.syy - s.sy * s.sy;
                if dx <= 0.0 {
                    self.error = "Error: x values are all equal".into();
                    return;
                }
                let b = (n * s.sxy - s.sx * s.sy) / dx;
                let a = (s.sy - b * s.sx) / n;
                let r = if dy > 0.0 {
                    (n * s.sxy - s.sx * s.sy) / (dx * dy).sqrt()
                } else {
                    0.0
                };
                let est = a + b * self.x();
                self.pair(est, r);
            }
        }
    }

    fn accumulate(&mut self, sign: f64) {
        let (x, y) = (self.stack[0], self.stack[1]);
        let s = &mut self.stats;
        s.n += sign;
        s.sx += sign * x;
        s.sxx += sign * x * x;
        s.sy += sign * y;
        s.syy += sign * y * y;
        s.sxy += sign * x * y;
        if s.n <= 0.0 {
            self.stats = Stats::default();
        }
        self.stack[0] = self.stats.n;
        self.lift = false;
    }

    fn press(&mut self, key: &str) {
        self.error.clear();
        let prefix = std::mem::take(&mut self.prefix);
        if prefix != Prefix::None {
            if let Ok(d) = key.parse::<usize>() {
                self.memory(prefix, d);
                return;
            }
            if let (Prefix::Rcl, Some(r)) = (prefix, TVM_NAMES.iter().position(|n| *n == key)) {
                self.put(self.tvm[r]);
                self.storable = true;
                return;
            }
            // Anything else cancels the prefix and works as usual.
        }
        let typed = self.storable;
        match key {
            "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "dot" => {
                return self.type_char(key)
            }
            "chs" => return self.change_sign(),
            "bksp" => return self.backspace(),
            _ => {}
        }
        self.commit();
        self.storable = false;
        match key {
            "enter" => {
                self.push_up();
                self.lift = false;
                self.storable = true;
            }
            "swap" => {
                self.stack.swap(0, 1);
                self.lift = true;
            }
            "roll" => {
                self.stack.rotate_left(1);
                self.lift = true;
            }
            "clx" => {
                self.stack[0] = 0.0;
                self.lift = false;
            }
            "clfin" => self.tvm = [0.0; 5],
            "clreg" => {
                *self = Finance {
                    decimals: self.decimals,
                    ..Default::default()
                }
            }
            "sto" => self.prefix = Prefix::Sto,
            "rcl" => self.prefix = Prefix::Rcl,
            "add" => self.binary(|y, x| Some(y + x)),
            "sub" => self.binary(|y, x| Some(y - x)),
            "mul" => self.binary(|y, x| Some(y * x)),
            "div" => self.binary(|y, x| (x != 0.0).then(|| y / x)),
            "pow" => self.binary(|y, x| Some(y.powf(x))),
            "inv" => self.unary(|x| (x != 0.0).then(|| 1.0 / x)),
            "sqrt" => self.unary(|x| (x >= 0.0).then(|| x.sqrt())),
            "sq" => self.unary(|x| Some(x * x)),
            "exp" => self.unary(|x| Some(x.exp())),
            "ln" => self.unary(|x| (x > 0.0).then(|| x.ln())),
            "pi" => self.put(std::f64::consts::PI),
            // The percent keys keep Y, so that a result can be used again.
            "pct" => {
                let [x, y, ..] = self.stack;
                self.result(y * x / 100.0);
            }
            "dpct" => {
                let [x, y, ..] = self.stack;
                if y == 0.0 {
                    self.error = "Error: Y is zero".into();
                } else {
                    self.result((x - y) / y * 100.0);
                }
            }
            "pctt" => {
                let [x, y, ..] = self.stack;
                if y == 0.0 {
                    self.error = "Error: Y is zero".into();
                } else {
                    self.result(x / y * 100.0);
                }
            }
            "n" | "i" | "pv" | "pmt" | "fv" => {
                let which = TVM_NAMES.iter().position(|n| *n == key).expect("a TVM key");
                self.tvm_key(which, typed);
            }
            "12x" => {
                self.tvm[N] = self.x() * 12.0;
                self.result(self.tvm[N]);
            }
            "12/" => {
                self.tvm[I] = self.x() / 12.0;
                self.result(self.tvm[I]);
            }
            "beg" => self.begin = true,
            "end" => self.begin = false,
            "cf0" => {
                self.cf0 = self.x();
                self.flows.clear();
                self.lift = true;
            }
            "cfj" => {
                if self.flows.len() >= 80 {
                    self.error = "Error: too many cash flows".into();
                } else {
                    self.flows.push((self.x(), 1));
                    self.lift = true;
                }
            }
            "nj" => match self.flows.last_mut() {
                Some(last) if (1.0..=99.0).contains(&self.stack[0]) => {
                    last.1 = self.stack[0] as u32;
                    self.lift = true;
                }
                Some(_) => self.error = "Error: Nj must be 1 to 99".into(),
                None => self.error = "Error: no cash flow yet".into(),
            },
            "npv" => {
                let v = self.npv_at(self.tvm[I] / 100.0);
                self.result(v);
            }
            "irr" => match solve(|r| self.npv_at(r)) {
                Some(r) => self.result(r * 100.0),
                None => self.error = "Error: no IRR".into(),
            },
            "sum+" => self.accumulate(1.0),
            "sum-" => self.accumulate(-1.0),
            "mean" | "sdev" | "yhat" => self.statistic(key),
            _ => {}
        }
    }

    // --- display ---------------------------------------------------------

    fn show(&self, out: &mut Outbox) {
        let d = self.decimals;
        let x = match &self.entry {
            Some(e) if e.is_empty() => "0".to_owned(),
            Some(e) => e.clone(),
            None => fixed(self.x(), d),
        };
        let prefix = match self.prefix {
            Prefix::None => "",
            Prefix::Sto => "STO _",
            Prefix::Rcl => "RCL _",
        };
        out.signal(json!({
            "lcd": {
                "t": format!("T  {}", fixed(self.stack[3], d)),
                "z": format!("Z  {}", fixed(self.stack[2], d)),
                "y": format!("Y  {}", fixed(self.stack[1], d)),
                "x": x,
            },
            "mode": if self.begin { "BEGIN" } else { "END" },
            "prefix": prefix,
            "error": self.error,
            "tvm": {
                "n": exact(self.tvm[N]),
                "i": exact(self.tvm[I]),
                "pv": exact(self.tvm[PV]),
                "pmt": exact(self.tvm[PMT]),
                "fv": exact(self.tvm[FV]),
            },
            "memory": format!(
                "CF0 {} · {} flows · Σ n={}",
                exact(self.cf0),
                self.flows.len(),
                exact(self.stats.n)
            ),
        }));
    }
}

impl App for Finance {
    fn hello(&mut self, out: &mut Outbox, _: &Map<String, Value>) {
        self.decimals = 2;
        out.signal(json!({"settings": {"decimals": 2}}));
        self.show(out);
        out.element(ui::column("main").gap("large"));
        out.element(
            ui::text("title")
                .parent("main")
                .variant("h1")
                .text("Capital Twelve — Financial Calculator"),
        );
        out.element(
            ui::text("help")
                .parent("main")
                .variant("caption")
                .text(
                    "RPN: type a number, ENTER, type another, then an operator. \
                     Type a number then n, i, PV, PMT or FV to store it; press a key alone to solve for it.",
                ),
        );
        out.element(ui::card("display").parent("main").gap("small"));
        for (id, path) in [("t", "t"), ("z", "z"), ("y", "y")] {
            out.element(
                ui::text(format!("line_{id}"))
                    .parent("display")
                    .variant("code")
                    .text(cel(format!("s.lcd.{path}"))),
            );
        }
        out.element(
            ui::text("line_x")
                .parent("display")
                .variant("h1")
                .text(cel("s.lcd.x")),
        );
        out.element(ui::row("status").parent("display").gap("small").wrap(true));
        out.element(
            ui::badge("mode")
                .parent("status")
                .variant("info")
                .text(cel("s.mode")),
        );
        out.element(
            ui::badge("prefix")
                .parent("status")
                .variant("warning")
                .visible(cel("s.prefix != ''"))
                .text(cel("s.prefix")),
        );
        for name in TVM_NAMES {
            out.element(
                ui::badge(format!("reg_{name}"))
                    .parent("status")
                    .variant("success")
                    .text(cel(format!("'{} ' + s.tvm.{name}", name.to_uppercase()))),
            );
        }
        out.element(
            ui::text("memory")
                .parent("display")
                .variant("caption")
                .text(cel("s.memory")),
        );
        out.element(
            ui::notice("error")
                .parent("main")
                .variant("failure")
                .visible(cel("s.error != ''"))
                .text(cel("s.error")),
        );
        out.element(ui::card("keys").parent("main").title("Keys").gap("small"));
        for (r, row) in KEY_ROWS.iter().enumerate() {
            let row_id = format!("keys_{r}");
            out.element(ui::row(&row_id).parent("keys").gap("small").wrap(true));
            for (code, label, variant) in *row {
                out.element(
                    ui::button(format!("k_{code}"))
                        .parent(&row_id)
                        .label(*label)
                        .variant(*variant)
                        .on_press(Handler::action("key").payload(*code)),
                );
            }
        }
        out.element(
            ui::slider("decimals")
                .parent("keys")
                .label("Decimals shown")
                .min(0)
                .max(9)
                .bind("settings.decimals")
                .on_change(Handler::action("decimals")),
        );
    }

    fn action(&mut self, out: &mut Outbox, action: &Action) {
        if let Some(d) = action.signal("settings.decimals").and_then(Value::as_f64) {
            self.decimals = d.clamp(0.0, 9.0) as usize;
        }
        if action.name == "key" {
            if let Some(key) = action.payload.as_ref().and_then(Value::as_str) {
                self.press(key);
            }
        }
        self.show(out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hypereffect::server::Loopback;

    /// Presses space-separated keys, then returns the X display.
    fn keys(l: &mut Loopback<Finance>, keys: &str) -> String {
        for k in keys.split_whitespace() {
            assert!(l.trigger(&format!("k_{k}"), "onPress"), "no key {k}");
        }
        l.client
            .attr("line_x", "text")
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap()
    }

    fn new() -> Loopback<Finance> {
        Loopback::new()
    }

    #[test]
    fn every_key_is_a_button() {
        let l = new();
        let count: usize = KEY_ROWS.iter().map(|r| r.len()).sum();
        assert!(count > 50);
        for (code, ..) in KEY_ROWS.iter().flat_map(|r| r.iter()) {
            assert!(l.client.contains(&format!("k_{code}")), "{code}");
        }
    }

    #[test]
    fn stack_arithmetic() {
        let mut l = new();
        assert_eq!(keys(&mut l, "2 enter 3 add 4 mul"), "20.00");
        assert_eq!(keys(&mut l, "clreg 1 2 3 enter 4"), "4");
        assert_eq!(l.client.attr("line_y", "text"), Some(json!("Y  123.00")));
        assert_eq!(keys(&mut l, "sub"), "119.00");
        // Roll, swap and the stack's memory of T.
        assert_eq!(keys(&mut l, "clreg 1 enter 2 enter 3 enter 4 roll"), "3.00");
        assert_eq!(l.client.attr("line_t", "text"), Some(json!("T  4.00")));
        assert_eq!(keys(&mut l, "swap"), "2.00");
        // Negative numbers, decimals, backspace.
        assert_eq!(keys(&mut l, "clreg 5 dot 2 5 chs"), "-5.25");
        assert_eq!(keys(&mut l, "bksp"), "-5.2");
        assert_eq!(keys(&mut l, "enter 2 mul"), "-10.40");
        assert_eq!(keys(&mut l, "inv"), "-0.10");
        assert!(l.client.take_diagnostics().is_empty());
    }

    #[test]
    fn errors_leave_the_stack_alone() {
        let mut l = new();
        assert_eq!(keys(&mut l, "1 enter 0 div"), "0.00");
        assert_eq!(l.client.attr("error", "visible"), Some(json!(true)));
        // The next key clears the message; the operands are still there.
        assert_eq!(keys(&mut l, "swap"), "1.00");
        assert_eq!(l.client.attr("error", "visible"), Some(json!(false)));
        assert_eq!(keys(&mut l, "clx chs sqrt"), "0.00");
        assert_eq!(keys(&mut l, "1 chs sqrt"), "-1.00");
        assert_eq!(l.client.attr("error", "visible"), Some(json!(true)));
    }

    #[test]
    fn percent_keys() {
        let mut l = new();
        assert_eq!(keys(&mut l, "2 0 0 enter 1 5 pct"), "30.00");
        assert_eq!(keys(&mut l, "add"), "230.00");
        assert_eq!(keys(&mut l, "clreg 8 0 enter 1 0 0 dpct"), "25.00");
        assert_eq!(keys(&mut l, "clreg 2 0 0 enter 5 0 pctt"), "25.00");
    }

    #[test]
    fn memory_registers() {
        let mut l = new();
        assert_eq!(keys(&mut l, "4 2 sto 3 clx"), "0.00");
        assert_eq!(keys(&mut l, "rcl 3"), "42.00");
        assert_eq!(l.client.attr("prefix", "visible"), Some(json!(false)));
        keys(&mut l, "rcl");
        assert_eq!(l.client.value("prefix"), Some(&json!("RCL _")));
        // A non-digit cancels the prefix.
        keys(&mut l, "clx");
        assert_eq!(l.client.value("prefix"), Some(&json!("")));
    }

    #[test]
    fn loan_payment_and_friends() {
        let mut l = new();
        // 12 months at 1% a month on 1000.
        keys(&mut l, "1 2 n 1 i 1 0 0 0 pv");
        assert_eq!(keys(&mut l, "pmt"), "-88.85");
        assert_eq!(
            l.client.attr("reg_pmt", "text"),
            Some(json!("PMT -88.848789"))
        );
        // Solving for the others returns the inputs.
        assert_eq!(keys(&mut l, "clx n"), "12.00");
        assert_eq!(keys(&mut l, "clx i"), "1.00");
        assert_eq!(keys(&mut l, "clx pv"), "1,000.00");
        assert_eq!(keys(&mut l, "fv"), "0.00");
        // Compound growth: 1000 at 5% for 10 years.
        keys(&mut l, "clreg 1 0 n 5 i 1 0 0 0 chs pv");
        assert_eq!(keys(&mut l, "fv"), "1,628.89");
        // The annuity due pays a little less.
        keys(&mut l, "clreg 1 2 n 1 i 1 0 0 0 pv beg");
        assert_eq!(l.client.value("mode"), Some(&json!("BEGIN")));
        assert_eq!(keys(&mut l, "pmt"), "-87.97");
        // 12× and 12÷ convert yearly to monthly.
        assert_eq!(keys(&mut l, "clreg 3 0 12x"), "360.00");
        assert_eq!(keys(&mut l, "clx 6 12/"), "0.50");
        assert_eq!(l.client.value("tvm.n"), Some(&json!("360")));
        assert_eq!(l.client.value("tvm.i"), Some(&json!("0.5")));
        assert!(l.client.take_diagnostics().is_empty());
    }

    #[test]
    fn zero_rate_and_unsolvable() {
        let mut l = new();
        keys(&mut l, "1 0 n 1 0 0 0 pv");
        assert_eq!(keys(&mut l, "pmt"), "-100.00");
        // No rate gives a positive balance with only deposits of one sign.
        keys(&mut l, "clreg 1 0 n 1 0 0 pv 1 0 pmt");
        keys(&mut l, "clx i");
        assert_eq!(l.client.attr("error", "visible"), Some(json!(true)));
    }

    #[test]
    fn cash_flows() {
        let mut l = new();
        keys(&mut l, "1 0 0 0 chs cf0 5 0 0 cfj 3 nj");
        assert_eq!(
            l.client.value("memory"),
            Some(&json!("CF0 -1,000 · 1 flows · Σ n=0"))
        );
        assert_eq!(keys(&mut l, "1 0 i"), "10.00");
        // -1000 + 500 for three periods at 10%.
        assert_eq!(keys(&mut l, "clx npv"), "243.43");
        // At the IRR the net present value is nil.
        let irr = keys(&mut l, "irr");
        assert_eq!(irr, "23.38");
        assert_eq!(keys(&mut l, "clx nj"), "0.00");
        assert_eq!(l.client.attr("error", "visible"), Some(json!(true)));
    }

    #[test]
    fn statistics() {
        let mut l = new();
        // x values 2, 4, 6 with y = 10, 20, 30: a perfect line y = 5x.
        assert_eq!(keys(&mut l, "1 0 enter 2 sum+"), "1.00");
        keys(&mut l, "2 0 enter 4 sum+ 3 0 enter 6 sum+");
        assert_eq!(keys(&mut l, "mean"), "4.00");
        assert_eq!(l.client.attr("line_y", "text"), Some(json!("Y  20.00")));
        assert_eq!(keys(&mut l, "sdev"), "2.00");
        assert_eq!(l.client.attr("line_y", "text"), Some(json!("Y  10.00")));
        assert_eq!(keys(&mut l, "clx 8 yhat"), "40.00");
        assert_eq!(l.client.attr("line_y", "text"), Some(json!("Y  1.00")));
        // Σ− removes a pair that was added by mistake.
        assert_eq!(keys(&mut l, "clx 9 9 enter 9 sum+"), "4.00");
        assert_eq!(keys(&mut l, "clx 9 9 enter 9 sum-"), "3.00");
        assert_eq!(keys(&mut l, "clx mean"), "4.00");
        keys(&mut l, "clreg mean");
        assert_eq!(l.client.attr("error", "visible"), Some(json!(true)));
    }

    #[test]
    fn decimals_slider() {
        let mut l = new();
        keys(&mut l, "1 enter 3 div");
        l.write("settings.decimals", json!(5));
        assert!(l.trigger("decimals", "onChange"));
        assert_eq!(l.client.attr("line_x", "text"), Some(json!("0.33333")));
        l.write("settings.decimals", json!(0));
        assert!(l.trigger("decimals", "onChange"));
        assert_eq!(l.client.attr("line_x", "text"), Some(json!("0")));
        assert_eq!(fixed(1234567.891, 2), "1,234,567.89");
        assert_eq!(fixed(-0.001, 2), "0.00");
        assert!(l.client.take_diagnostics().is_empty());
    }
}
