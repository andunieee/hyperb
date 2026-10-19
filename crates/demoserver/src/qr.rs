//! The QR demo: type something, get a QR code as an image element.
//!
//! The server encodes the text, draws the modules into a 1-bit PNG and sends
//! it as a `data:` URI in a signal that the image element's `src` reads.

use super::common::{feedback, menu, number, text};
use super::img;
use hypereffect::{cel, json, ui, Action, App, Handler, Map, Outbox, Value};
use qrcode::{Color, EcLevel, QrCode};

/// Modules of white around the code, as the standard requires.
const QUIET: usize = 4;

#[derive(Default)]
pub struct Qr;

fn level(name: &str) -> EcLevel {
    match name {
        "L" => EcLevel::L,
        "Q" => EcLevel::Q,
        "H" => EcLevel::H,
        _ => EcLevel::M,
    }
}

/// The PNG of `data` at `scale` pixels per module, and the modules per side.
fn render(data: &str, ec: EcLevel, scale: usize) -> Result<(Vec<u8>, usize), String> {
    let code = QrCode::with_error_correction_level(data.as_bytes(), ec)
        .map_err(|e| format!("Cannot encode that: {e}."))?;
    let modules = code.width();
    let side = (modules + 2 * QUIET) * scale;
    let colors = code.to_colors();
    let mut white = vec![true; side * side];
    for (i, c) in colors.iter().enumerate() {
        if *c == Color::Dark {
            let (mx, my) = (i % modules + QUIET, i / modules + QUIET);
            for y in my * scale..(my + 1) * scale {
                for x in mx * scale..(mx + 1) * scale {
                    white[y * side + x] = false;
                }
            }
        }
    }
    Ok((img::png_bilevel(side as u32, side as u32, &white), modules))
}

impl App for Qr {
    fn hello(&mut self, out: &mut Outbox, _: &Map<String, Value>) {
        out.signal(json!({
            "form": {"text": "", "level": "M", "scale": 6},
            "menu": {"levels": menu(&[
                ("L", "Low (7%)"), ("M", "Medium (15%)"), ("Q", "Quartile (25%)"), ("H", "High (30%)"),
            ])},
            "qr": {"src": "", "info": ""},
            "feedback": {"kind": "info", "text": ""},
        }));
        out.element(ui::column("main").gap("large"));
        out.element(
            ui::text("title")
                .parent("main")
                .variant("h1")
                .text("Quadrat — QR Codes"),
        );
        out.element(
            ui::card("input")
                .parent("main")
                .title("What should it say?")
                .gap("medium"),
        );
        out.element(
            ui::input("text")
                .parent("input")
                .label("Text or link")
                .bind("form.text")
                .placeholder("https://example.com")
                .variant("multiline")
                .on_change(Handler::action("generate")),
        );
        out.element(
            ui::select("level")
                .parent("input")
                .label("Error correction")
                .bind("form.level")
                .options(cel("s.menu.levels"))
                .on_change(Handler::action("generate")),
        );
        out.element(
            ui::slider("scale")
                .parent("input")
                .label("Pixels per module")
                .bind("form.scale")
                .min(2)
                .max(12)
                .on_change(Handler::action("generate")),
        );
        out.element(
            ui::button("generate")
                .parent("input")
                .label("Generate")
                .variant("primary")
                .on_press(Handler::action("generate")),
        );
        out.element(
            ui::card("result")
                .parent("main")
                .title("Your code")
                .gap("medium"),
        );
        out.element(
            ui::text("empty")
                .parent("result")
                .visible(cel("s.qr.src == ''"))
                .text("Type something above."),
        );
        out.element(
            ui::image("code")
                .parent("result")
                .visible(cel("s.qr.src != ''"))
                .src(cel("s.qr.src"))
                .alt(cel("'QR code for: ' + s.form.text")),
        );
        out.element(
            ui::badge("info")
                .parent("result")
                .variant("info")
                .visible(cel("s.qr.info != ''"))
                .text(cel("s.qr.info")),
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
        if action.name != "generate" {
            return;
        }
        let data = action
            .signal("form.text")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if data.trim().is_empty() {
            out.signal(json!({"qr": {"src": "", "info": ""}}));
            out.signal(feedback("info", ""));
            return;
        }
        let scale = number(action, "form.scale").unwrap_or(6.0).clamp(2.0, 12.0) as usize;
        match render(data, level(&text(action, "form.level")), scale) {
            Ok((png, modules)) => {
                out.signal(json!({"qr": {
                    "src": img::data_uri(&png),
                    "info": format!("{modules}×{modules} modules, {} bytes of text", data.len()),
                }}));
                out.signal(feedback("info", ""));
            }
            Err(e) => {
                out.signal(json!({"qr": {"src": "", "info": ""}}));
                out.signal(feedback("failure", e));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hypereffect::server::Loopback;

    #[test]
    fn text_becomes_an_image() {
        let mut l = Loopback::<Qr>::new(Map::new());
        assert_eq!(l.client.attr("code", "visible"), Some(json!(false)));
        l.write("form.text", json!("https://example.com"));
        assert!(l.trigger("text", "onChange"));
        assert_eq!(l.client.attr("code", "visible"), Some(json!(true)));
        let src = l.client.attr("code", "src").unwrap();
        assert!(src.as_str().unwrap().starts_with("data:image/png;base64,"));
        assert_eq!(
            l.client.attr("info", "text"),
            Some(json!("25×25 modules, 19 bytes of text"))
        );
        // A bigger scale makes a bigger picture.
        l.write("form.scale", json!(12));
        assert!(l.trigger("scale", "onChange"));
        assert_ne!(l.client.attr("code", "src").unwrap(), src);
        // Clearing the text clears the image.
        l.write("form.text", json!(""));
        assert!(l.trigger("generate", "onPress"));
        assert_eq!(l.client.attr("code", "visible"), Some(json!(false)));
        assert!(l.client.take_diagnostics().is_empty());
    }

    #[test]
    fn too_much_text_is_reported() {
        let mut l = Loopback::<Qr>::new(Map::new());
        l.write("form.text", json!("x".repeat(4000)));
        assert!(l.trigger("generate", "onPress"));
        assert_eq!(l.client.attr("feedback", "visible"), Some(json!(true)));
        assert_eq!(l.client.attr("code", "visible"), Some(json!(false)));
        assert!(l.client.take_diagnostics().is_empty());
    }

    #[test]
    fn png_has_the_expected_size() {
        let (png, modules) = render("hi", EcLevel::L, 3).unwrap();
        let side = ((modules + 8) * 3) as u32;
        assert_eq!(&png[16..20], &side.to_be_bytes());
        assert_eq!(&png[20..24], &side.to_be_bytes());
    }
}
