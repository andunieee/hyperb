//! Helpers shared by the newer demos.

use hypereffect::{json, Action, Element, Outbox, Value};

/// Replaces the elements `ids` with `new`: the old ones are removed and the
/// ids of the new ones remembered, for a part of the interface that is drawn
/// again as a whole whenever its data changes.
pub fn replace(out: &mut Outbox, ids: &mut Vec<String>, new: Vec<Element>) {
    for id in ids.drain(..) {
        out.remove(id);
    }
    for e in new {
        ids.push(e.id.clone());
        out.element(e);
    }
}

/// A signal as trimmed text; numbers are written out.
pub fn text(action: &Action, path: &str) -> String {
    match action.signal(path) {
        Some(Value::String(s)) => s.trim().to_owned(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

/// A signal as a number.
pub fn number(action: &Action, path: &str) -> Option<f64> {
    action.signal(path)?.as_f64()
}

/// A select menu in the component's shape.
pub fn menu(values: &[(&str, &str)]) -> Vec<Value> {
    values
        .iter()
        .map(|(v, l)| json!({"value": v, "label": l}))
        .collect()
}

/// The `feedback` signal patch for a notice.
pub fn feedback(kind: &str, text: impl Into<String>) -> Value {
    json!({"feedback": {"kind": kind, "text": text.into()}})
}

/// An amount such as `12.50`, `$1,200` or `12,5` in cents. Both `.` and `,`
/// may be the decimal mark; a lone comma followed by three digits is a
/// thousands separator.
pub fn parse_cents(s: &str) -> Option<i64> {
    let t: String = s
        .chars()
        .filter(|c| !c.is_whitespace() && !"$€£¥".contains(*c))
        .collect();
    let lone_decimal_comma =
        t.matches(',').count() == 1 && !t.contains('.') && t.rsplit(',').next()?.len() != 3;
    let t = if lone_decimal_comma {
        t.replace(',', ".")
    } else {
        t.replace(',', "")
    };
    let v = t.parse::<f64>().ok().filter(|v| v.abs() < 1e12)?;
    Some((v * 100.0).round() as i64)
}

/// Digits with a comma between each group of three.
pub fn group(digits: &str) -> String {
    let bytes = digits.as_bytes();
    let first = match bytes.len() % 3 {
        0 => 3.min(bytes.len()),
        r => r,
    };
    let mut out = String::from_utf8_lossy(&bytes[..first]).into_owned();
    for chunk in bytes[first..].chunks(3) {
        out.push(',');
        out.push_str(&String::from_utf8_lossy(chunk));
    }
    out
}

/// Money in cents as `1,234.56`, with a leading `-` when negative.
pub fn money(cents: i64) -> String {
    let abs = cents.unsigned_abs();
    let whole = (abs / 100).to_string();
    let grouped = group(&whole);
    format!(
        "{}{grouped}.{:02}",
        if cents < 0 { "-" } else { "" },
        abs % 100
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn amounts_parse() {
        assert_eq!(parse_cents("12.50"), Some(1250));
        assert_eq!(parse_cents(" $1,200 "), Some(120000));
        assert_eq!(parse_cents("12,5"), Some(1250));
        assert_eq!(parse_cents("1,234.56"), Some(123456));
        assert_eq!(parse_cents("-3"), Some(-300));
        assert_eq!(parse_cents("abc"), None);
        assert_eq!(parse_cents("inf"), None);
        assert_eq!(parse_cents(""), None);
    }

    #[test]
    fn grouping() {
        assert_eq!(group(""), "");
        assert_eq!(group("12"), "12");
        assert_eq!(group("123"), "123");
        assert_eq!(group("1234"), "1,234");
        assert_eq!(group("123456"), "123,456");
        assert_eq!(group("1234567"), "1,234,567");
    }

    #[test]
    fn money_formats() {
        assert_eq!(money(0), "0.00");
        assert_eq!(money(5), "0.05");
        assert_eq!(money(123456), "1,234.56");
        assert_eq!(money(-100000000), "-1,000,000.00");
    }
}
