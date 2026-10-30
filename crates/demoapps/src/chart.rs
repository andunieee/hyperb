//! The chart demo: type some numbers, get a chart as an image element.
//!
//! The data is free text, one `label, value` pair per line (or a single line
//! of plain numbers). The server draws a bar, line or pie chart on a canvas,
//! with a built-in bitmap font so it needs no fonts installed, encodes it as
//! a PNG and sends it as a `data:` URI.

use super::common::{feedback, menu, text};
use super::img::{self, Canvas, Rgb, text_width};
use hypereffect::{Action, App, Handler, Outbox, cel, json, ui};
use std::collections::BTreeMap;

const W: i32 = 640;
const H: i32 = 400;
/// The most points a chart takes.
const MAX_POINTS: usize = 24;

const BACKGROUND: Rgb = [255, 255, 255];
const INK: Rgb = [0x33, 0x33, 0x33];
const GRID: Rgb = [0xDD, 0xDD, 0xDD];
pub const PALETTE: [Rgb; 8] = [
    [0x25, 0x63, 0xEB],
    [0xF5, 0x9E, 0x0B],
    [0x16, 0xA3, 0x4A],
    [0xDC, 0x26, 0x26],
    [0x7C, 0x3A, 0xED],
    [0x0D, 0x94, 0x88],
    [0xDB, 0x27, 0x77],
    [0x6B, 0x72, 0x80],
];
const COLORS: &[(&str, &str)] = &[
    ("blue", "Blue"),
    ("orange", "Orange"),
    ("green", "Green"),
    ("red", "Red"),
    ("purple", "Purple"),
    ("teal", "Teal"),
];
const SAMPLE: &str = "Mon, 12\nTue, 19\nWed, 7\nThu, 15\nFri, 22\nSat, 9";

#[derive(Default)]
pub struct Chart;

#[derive(Debug, PartialEq)]
pub struct Point {
    pub label: String,
    pub value: f64,
}

fn number(s: &str) -> Option<f64> {
    let s: String = s.trim().chars().filter(|c| !"$%€£ ".contains(*c)).collect();
    s.parse::<f64>().ok().filter(|v| v.is_finite())
}

/// Reads the data text into points.
fn parse(data: &str) -> Result<Vec<Point>, String> {
    let lines: Vec<&str> = data
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let mut points = Vec::new();
    if let [only] = lines.as_slice() {
        let nums: Vec<f64> = only
            .split(|c: char| c == ',' || c == ';' || c.is_whitespace())
            .filter(|t| !t.is_empty())
            .map_while(number)
            .collect();
        let tokens = only
            .split(|c: char| c == ',' || c == ';' || c.is_whitespace())
            .filter(|t| !t.is_empty())
            .count();
        if nums.len() >= 2 && nums.len() == tokens {
            points = nums
                .into_iter()
                .enumerate()
                .map(|(i, value)| Point {
                    label: (i + 1).to_string(),
                    value,
                })
                .collect();
        }
    }
    if points.is_empty() {
        for (i, line) in lines.iter().enumerate() {
            let split = line
                .rsplit_once([',', ':', '=', ';', '\t'])
                .or_else(|| line.rsplit_once(char::is_whitespace));
            let point = match split {
                Some((label, value)) => number(value).map(|value| Point {
                    label: label.trim().to_owned(),
                    value,
                }),
                None => number(line).map(|value| Point {
                    label: (i + 1).to_string(),
                    value,
                }),
            };
            match point {
                Some(p) => points.push(p),
                None => return Err(format!("Line {} has no number: \"{line}\".", i + 1)),
            }
        }
    }
    if points.is_empty() {
        return Err("Enter some data, one \"label, value\" per line.".into());
    }
    if points.len() > MAX_POINTS {
        return Err(format!("At most {MAX_POINTS} values fit in a chart."));
    }
    Ok(points)
}

fn fmt(v: f64) -> String {
    if (v - v.round()).abs() < 1e-9 {
        format!("{}", v.round() as i64)
    } else {
        let s = format!("{v:.2}");
        s.trim_end_matches('0').trim_end_matches('.').to_owned()
    }
}

/// A step of 1, 2 or 5 times a power of ten near `raw`.
fn nice_step(raw: f64) -> f64 {
    let mag = 10f64.powf(raw.log10().floor());
    let f = raw / mag;
    let n = if f <= 1.0 {
        1.0
    } else if f <= 2.0 {
        2.0
    } else if f <= 5.0 {
        5.0
    } else {
        10.0
    };
    n * mag
}

fn shorten(s: &str, chars: usize) -> String {
    if s.chars().count() <= chars {
        s.to_owned()
    } else {
        let mut out: String = s.chars().take(chars.saturating_sub(1)).collect();
        out.push('.');
        out
    }
}

fn centered(c: &mut Canvas, cx: i32, y: i32, scale: i32, s: &str, color: Rgb) {
    c.text(cx - text_width(s, scale) / 2, y, scale, s, color);
}

pub fn draw(kind: &str, title: &str, points: &[Point], tint: Rgb) -> Result<Vec<u8>, String> {
    let mut c = Canvas::new(W, H, BACKGROUND);
    if !title.is_empty() {
        centered(&mut c, W / 2, 14, 2, &shorten(title, 48), INK);
    }
    match kind {
        "pie" => pie(&mut c, points)?,
        "line" => axes(&mut c, points, tint, true),
        _ => axes(&mut c, points, tint, false),
    }
    Ok(c.png())
}

fn axes(c: &mut Canvas, points: &[Point], tint: Rgb, line: bool) {
    let (x0, x1, y0, y1) = (70, W - 20, 50, H - 50);
    let vmin = points.iter().map(|p| p.value).fold(0.0, f64::min);
    let vmax = points.iter().map(|p| p.value).fold(0.0, f64::max);
    let span = if vmax > vmin { vmax - vmin } else { 1.0 };
    let step = nice_step(span / 5.0);
    let lo = (vmin / step).floor() * step;
    let hi = ((vmax / step).ceil() * step).max(lo + step);
    let y = |v: f64| y1 - ((v - lo) / (hi - lo) * f64::from(y1 - y0)).round() as i32;
    let mut tick = lo;
    while tick <= hi + step / 1e6 {
        let ty = y(tick);
        c.line((x0, ty), (x1, ty), 1, GRID);
        let label = fmt(tick);
        c.text(x0 - 8 - text_width(&label, 1), ty - 3, 1, &label, INK);
        tick += step;
    }
    c.line((x0, y0), (x0, y1), 1, INK);
    c.line((x0, y(0.0)), (x1, y(0.0)), 1, INK);

    let slot = (x1 - x0) / points.len() as i32;
    let mut previous = None;
    for (i, p) in points.iter().enumerate() {
        let cx = x0 + slot * i as i32 + slot / 2;
        let top = y(p.value);
        let label = fmt(p.value);
        let above = p.value >= 0.0;
        if line {
            if let Some(prev) = previous {
                c.line(prev, (cx, top), 3, tint);
            }
            c.disc(cx, top, 5, tint);
            previous = Some((cx, top));
        } else {
            let half = (slot * 7 / 10).max(2) / 2;
            let (a, b) = (top.min(y(0.0)), top.max(y(0.0)));
            c.rect(cx - half, a, half * 2, (b - a).max(1), tint);
        }
        if text_width(&label, 1) <= slot {
            let gap = if line { 8 } else { 0 };
            let ly = if above { top - 12 - gap } else { top + 6 + gap };
            centered(c, cx, ly, 1, &label, INK);
        }
        let fit = (slot / 6).max(1) as usize;
        centered(c, cx, y1 + 10, 1, &shorten(&p.label, fit), INK);
    }
}

fn pie(c: &mut Canvas, points: &[Point]) -> Result<(), String> {
    if points.iter().any(|p| p.value < 0.0) {
        return Err("A pie chart needs values of zero or more.".into());
    }
    let sum: f64 = points.iter().map(|p| p.value).sum();
    if sum <= 0.0 {
        return Err("A pie chart needs a value above zero.".into());
    }
    let (cx, cy, r) = (190, 215, 130);
    let mut from = 0.0;
    for (i, p) in points.iter().enumerate() {
        let to = from + p.value / sum * std::f64::consts::TAU;
        c.slice(cx, cy, r, from, to + 1e-9, PALETTE[i % PALETTE.len()]);
        from = to;
    }
    let row = ((H - 90) / points.len() as i32).clamp(12, 24);
    for (i, p) in points.iter().enumerate() {
        let y = 60 + row * i as i32;
        c.rect(360, y, 10, 10, PALETTE[i % PALETTE.len()]);
        let pct = fmt((p.value / sum * 1000.0).round() / 10.0);
        c.text(
            378,
            y + 1,
            1,
            &format!("{} ({pct}%)", shorten(&p.label, 28)),
            INK,
        );
    }
    Ok(())
}

fn tint(name: &str) -> Rgb {
    let i = COLORS.iter().position(|(v, _)| *v == name).unwrap_or(0);
    PALETTE[i]
}

fn render(action_text: impl Fn(&str) -> String) -> Result<(String, String), String> {
    let points = parse(&action_text("form.data"))?;
    let title = action_text("form.title");
    let kind = action_text("form.kind");
    let png = draw(&kind, &title, &points, tint(&action_text("form.color")))?;
    let sum: f64 = points.iter().map(|p| p.value).sum();
    let info = format!("{} values, total {}", points.len(), fmt(sum));
    Ok((img::data_uri(&png), info))
}

impl Chart {
    fn show(&self, out: &mut Outbox, result: Result<(String, String), String>) {
        match result {
            Ok((src, info)) => {
                out.signal(json!({"chart": {"src": src, "info": info}}));
                out.signal(feedback("info", ""));
            }
            Err(e) => {
                out.signal(json!({"chart": {"src": "", "info": ""}}));
                out.signal(feedback("failure", e));
            }
        }
    }
}

impl App for Chart {
    fn hello(&mut self, out: &mut Outbox, _: &BTreeMap<String, String>) {
        let form =
            json!({"title": "Coffees per day", "data": SAMPLE, "kind": "bar", "color": "blue"});
        out.signal(json!({
            "form": form,
            "menu": {
                "kinds": menu(&[("bar", "Bar chart"), ("line", "Line chart"), ("pie", "Pie chart")]),
                "colors": menu(COLORS),
            },
            "chart": {"src": "", "info": ""},
            "feedback": {"kind": "info", "text": ""},
        }));
        let sample = |path: &str| match path {
            "form.title" => "Coffees per day".to_owned(),
            "form.data" => SAMPLE.to_owned(),
            "form.kind" => "bar".to_owned(),
            _ => "blue".to_owned(),
        };
        self.show(out, render(sample));
        out.element(ui::column("main").gap("large"));
        out.element(
            ui::text("title")
                .parent("main")
                .variant("h1")
                .text("Plotline — Chart Maker"),
        );
        out.element(
            ui::card("input")
                .parent("main")
                .title("Your data")
                .gap("medium"),
        );
        out.element(
            ui::input("name")
                .parent("input")
                .label("Title")
                .bind("form.title")
                .on_change(Handler::action("render")),
        );
        out.element(
            ui::input("data")
                .parent("input")
                .label("One \"label, value\" per line")
                .bind("form.data")
                .variant("multiline")
                .placeholder("Mon, 12\nTue, 19")
                .on_change(Handler::action("render")),
        );
        out.element(
            ui::select("kind")
                .parent("input")
                .label("Kind")
                .bind("form.kind")
                .variant("segmented")
                .options(cel("s.menu.kinds"))
                .on_change(Handler::action("render")),
        );
        out.element(
            ui::select("color")
                .parent("input")
                .label("Color (bar and line)")
                .bind("form.color")
                .options(cel("s.menu.colors"))
                .visible(cel("s.form.kind != 'pie'"))
                .on_change(Handler::action("render")),
        );
        out.element(
            ui::button("render")
                .parent("input")
                .label("Draw")
                .variant("primary")
                .on_press(Handler::action("render")),
        );
        out.element(
            ui::card("result")
                .parent("main")
                .title("Your chart")
                .gap("medium"),
        );
        out.element(
            ui::text("empty")
                .parent("result")
                .visible(cel("s.chart.src == ''"))
                .text("Nothing to draw yet."),
        );
        out.element(
            ui::image("picture")
                .parent("result")
                .visible(cel("s.chart.src != ''"))
                .src(cel("s.chart.src"))
                .alt(cel("s.form.kind + ' chart: ' + s.form.title")),
        );
        out.element(
            ui::badge("info")
                .parent("result")
                .variant("info")
                .visible(cel("s.chart.info != ''"))
                .text(cel("s.chart.info")),
        );
        out.element(
            ui::notice("feedback")
                .parent("main")
                .variant(cel("s.feedback.kind"))
                .visible(cel("s.feedback.text != ''"))
                .text(cel("s.feedback.text")),
        );
    }

    fn action(&mut self, out: &mut Outbox, action: &Action) {
        if action.name == "render" {
            self.show(out, render(|path| text(action, path)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hypereffect::server::Loopback;

    fn pts(data: &str) -> Vec<(String, f64)> {
        parse(data)
            .unwrap()
            .into_iter()
            .map(|p| (p.label, p.value))
            .collect()
    }

    #[test]
    fn parses_pairs_and_series() {
        assert_eq!(
            pts("Rent: 1200\nFood, 300.5\nBig Fun 40%"),
            [
                ("Rent".into(), 1200.0),
                ("Food".into(), 300.5),
                ("Big Fun".into(), 40.0)
            ]
        );
        assert_eq!(
            pts("3, 5 8"),
            [("1".into(), 3.0), ("2".into(), 5.0), ("3".into(), 8.0)]
        );
        assert_eq!(pts("7\n9"), [("1".into(), 7.0), ("2".into(), 9.0)]);
        assert!(parse("").is_err());
        assert!(parse("a, b").unwrap_err().starts_with("Line 1"));
        assert!(parse(&"x, 1\n".repeat(30)).is_err());
    }

    #[test]
    fn steps_and_numbers() {
        assert_eq!(nice_step(3.0), 5.0);
        assert_eq!(nice_step(0.12), 0.2);
        assert_eq!(fmt(2.0), "2");
        assert_eq!(fmt(2.5), "2.5");
        assert_eq!(fmt(1.0 / 3.0), "0.33");
    }

    #[test]
    fn draws_all_kinds() {
        let mut l = Loopback::<Chart>::new();
        // Drawn from the start.
        assert_eq!(l.client.attr("picture", "visible"), Some(json!(true)));
        assert_eq!(
            l.client.attr("info", "text"),
            Some(json!("6 values, total 84"))
        );
        let bar = l.client.attr("picture", "src").unwrap();
        for kind in ["line", "pie"] {
            l.write("form.kind", json!(kind));
            assert!(l.trigger("kind", "onChange"));
            let src = l.client.attr("picture", "src").unwrap();
            assert!(src.as_str().unwrap().starts_with("data:image/png;base64,"));
            assert_ne!(src, bar);
        }
        assert_eq!(l.client.attr("color", "visible"), Some(json!(false)));
        // Negative values suit a bar chart but not a pie.
        l.write("form.data", json!("a, -3\nb, 4"));
        assert!(l.trigger("render", "onPress"));
        assert_eq!(l.client.attr("picture", "visible"), Some(json!(false)));
        assert_eq!(l.client.attr("feedback", "visible"), Some(json!(true)));
        l.write("form.kind", json!("bar"));
        assert!(l.trigger("kind", "onChange"));
        assert_eq!(l.client.attr("picture", "visible"), Some(json!(true)));
        assert!(l.client.take_diagnostics().is_empty());
    }
}
