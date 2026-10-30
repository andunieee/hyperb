//! Draws a hypereffect element tree with Leptos.
//!
//! The [`Client`] keeps the tree, the signals and the expressions; each
//! element is a component holding a memo of its resolved attributes, so only
//! what changed is touched in the DOM. Children are keyed by id (and type),
//! which keeps their nodes, and so focus and selection, across updates.
//!
//! Every element is wrapped in a `div.he-el` with `data-type` and
//! `data-variant`; the stylesheet lays out and themes those wrappers.

use crate::browser::Ctx;
use hyperb_core::hypereffect::client::Client;
use hyperb_core::hypereffect::{Component, Map, Value, json};
use leptos::html;
use leptos::prelude::*;
use std::time::Duration;

/// How long an input waits after the last keystroke before running onChange.
const CHANGE_IDLE: Duration = Duration::from_millis(700);

/// The children of `parent` (the root if `None`).
#[component]
pub fn Children(ctx: Ctx, parent: Option<String>) -> impl IntoView {
    let kids = Memo::new(move |_| {
        ctx.with(|c| {
            c.children(parent.as_deref())
                .iter()
                .filter_map(|id| Some((id.clone(), c.get(id)?.component)))
                .collect::<Vec<_>>()
        })
    });
    view! {
        <For
            each=move || kids.get()
            key=|k| k.clone()
            children=move |(id, component)| view! { <Element ctx id component/> }
        />
    }
}

/// An element and its resolved attributes.
#[derive(Clone, Copy)]
struct El {
    ctx: Ctx,
    id: StoredValue<String>,
    attrs: Memo<Map<String, Value>>,
}

impl El {
    fn new(ctx: Ctx, id: String) -> Self {
        let sid = StoredValue::new(id.clone());
        let attrs =
            Memo::new(move |_| ctx.with(|c| c.get(&id).map(|r| r.attrs).unwrap_or_default()));
        El {
            ctx,
            id: sid,
            attrs,
        }
    }

    fn id(self) -> String {
        self.id.get_value()
    }

    fn s(self, k: &str) -> Option<String> {
        self.attrs
            .with(|a| a.get(k).and_then(Value::as_str).map(str::to_string))
    }

    fn text(self, k: &str) -> String {
        self.s(k).unwrap_or_default()
    }

    /// Whether the string attribute `k` is missing or empty.
    fn blank(self, k: &str) -> bool {
        self.attrs
            .with(|a| a.get(k).and_then(Value::as_str).is_none_or(str::is_empty))
    }

    fn b(self, k: &str) -> bool {
        self.attrs.with(|a| a.get(k) == Some(&Value::Bool(true)))
    }

    fn n(self, k: &str) -> Option<f64> {
        self.attrs.with(|a| a.get(k).and_then(Value::as_f64))
    }

    fn has(self, k: &str) -> bool {
        self.attrs.with(|a| !a.get(k).is_none_or(Value::is_null))
    }

    /// The value of the signal in `bind`. Without `bind` the control keeps
    /// its own state, so callers read the DOM instead.
    fn bound(self) -> Option<Value> {
        let path = self.s("bind")?;
        self.ctx.with(|c| c.value(&path).cloned())
    }

    fn disabled(self) -> bool {
        self.b("disabled") || !self.ctx.connected()
    }

    fn input(self, value: Option<Value>, event: Option<&str>) {
        self.ctx.input(&self.id(), value, event)
    }

    fn dom_id(self) -> String {
        format!("he-{}", self.id())
    }
}

#[component]
fn Element(ctx: Ctx, id: String, component: Component) -> impl IntoView {
    let el = El::new(ctx, id);
    let hidden = move || match component {
        Component::Dialog => !(el.b("visible") && el.b("open")),
        Component::Tab => !el.b("visible") || !tab_shown(ctx, &el.id()),
        _ => !el.b("visible"),
    };
    let inner = match component {
        Component::Column => stack(el, "medium").into_any(),
        Component::Tab => stack(el, "medium").into_any(),
        Component::Row => row(el).into_any(),
        Component::Card => card(el).into_any(),
        Component::List => list(el).into_any(),
        Component::Tabs => tabs(el).into_any(),
        Component::Dialog => dialog(el).into_any(),
        Component::Text => text(el).into_any(),
        Component::Notice => notice(el).into_any(),
        Component::Badge => {
            view! { <span class="he-badge">{move || el.text("text")}</span> }.into_any()
        }
        Component::Image => image(el).into_any(),
        Component::Divider => view! { <hr class="he-divider"/> }.into_any(),
        Component::Progress => progress(el).into_any(),
        Component::Button => button(el).into_any(),
        Component::Input => input(el).into_any(),
        Component::Checkbox => checkbox(el).into_any(),
        Component::Select => select(el).into_any(),
        Component::Slider => slider(el).into_any(),
    };
    view! {
        <div
            class="he-el"
            data-type=component.name()
            data-variant=move || el.s("variant")
            class:he-hidden=hidden
        >
            {inner}
        </div>
    }
}

// --- containers --------------------------------------------------------------

fn stack(el: El, gap: &'static str) -> impl IntoView {
    view! {
        <div
            class="he-column"
            data-gap=move || el.s("gap").unwrap_or(gap.into())
            data-align=move || el.s("align").unwrap_or("stretch".into())
        >
            <Children ctx=el.ctx parent=Some(el.id())/>
        </div>
    }
}

fn row(el: El) -> impl IntoView {
    view! {
        <div
            class="he-row"
            data-gap=move || el.s("gap").unwrap_or("small".into())
            data-align=move || el.s("align").unwrap_or("center".into())
            data-justify=move || el.s("justify").unwrap_or("start".into())
        >
            <Children ctx=el.ctx parent=Some(el.id())/>
        </div>
    }
}

fn card(el: El) -> impl IntoView {
    view! {
        <section class="he-card">
            <h3 class="he-card-title" class:he-hidden=move || el.blank("title")>
                {move || el.text("title")}
            </h3>
            {stack(el, "medium")}
        </section>
    }
}

fn list(el: El) -> impl IntoView {
    let ctx = el.ctx;
    let any_shown = move || {
        let id = el.id();
        ctx.with(|c| {
            c.children(Some(&id))
                .iter()
                .any(|k| c.attr(k, "visible") == Some(Value::Bool(true)))
        })
    };
    view! {
        <div class="he-list">
            <div class="he-list-items he-column" data-gap="none" data-align="stretch">
                <Children ctx parent=Some(el.id())/>
            </div>
            <p class="he-list-empty" class:he-hidden=move || el.blank("empty") || any_shown()>
                {move || el.text("empty")}
            </p>
        </div>
    }
}

/// The tab a `tabs` element shows: its bound (or locally chosen) tab if that
/// is visible, else the first visible one.
pub fn selected_tab(c: &Client, tabs: &str, local: Option<&String>) -> Option<String> {
    let r = c.get(tabs)?;
    let shown: Vec<&String> = r
        .children
        .iter()
        .filter(|k| c.attr(k, "visible") == Some(Value::Bool(true)))
        .collect();
    let wanted = match r.attrs.get("bind").and_then(Value::as_str) {
        Some(path) => c.value(path).and_then(Value::as_str).map(str::to_string),
        None => local.cloned(),
    };
    wanted
        .filter(|w| shown.contains(&w))
        .or_else(|| shown.first().map(|s| s.to_string()))
}

/// Whether the tab `id` is the one its `tabs` parent shows.
fn tab_shown(ctx: Ctx, id: &str) -> bool {
    let local = ctx.local_tabs.get();
    ctx.with(|c| {
        let parent = c.get(id).and_then(|r| r.parent);
        parent.is_none_or(|p| selected_tab(c, &p, local.get(&p)).as_deref() == Some(id))
    })
}

fn tabs(el: El) -> impl IntoView {
    let ctx = el.ctx;
    let labels = Memo::new(move |_| {
        let id = el.id();
        ctx.with(|c| {
            c.children(Some(&id))
                .iter()
                .filter(|k| c.attr(k, "visible") == Some(Value::Bool(true)))
                .map(|k| {
                    let label = c
                        .attr(k, "label")
                        .and_then(|v| v.as_str().map(str::to_string));
                    (k.clone(), label.unwrap_or_else(|| k.clone()))
                })
                .collect::<Vec<_>>()
        })
    });
    let selected = Memo::new(move |_| {
        let (id, local) = (el.id(), ctx.local_tabs.get());
        ctx.with(|c| selected_tab(c, &id, local.get(&id)))
    });
    view! {
        <div class="he-tabs">
            <div class="he-tabbar" role="tablist">
                <For
                    each=move || labels.get()
                    key=|t| t.clone()
                    children=move |(tab, label)| {
                        let t = tab.clone();
                        view! {
                            <button
                                role="tab"
                                class="he-tab-button"
                                aria-selected=move || (selected.get().as_deref() == Some(&tab)).to_string()
                                on:click=move |_| {
                                    ctx.local_tabs.update(|m| {
                                        m.insert(el.id(), t.clone());
                                    });
                                    el.input(Some(json!(t)), Some("onChange"));
                                }
                            >
                                {label}
                            </button>
                        }
                    }
                />
            </div>
            <div class="he-column">
                <Children ctx parent=Some(el.id())/>
            </div>
        </div>
    }
}

fn dialog(el: El) -> impl IntoView {
    let dismissable = move || el.has("onDismiss");
    let dismiss = move || el.input(None, Some("onDismiss"));
    view! {
        <div
            class="he-backdrop"
            on:click=move |e| {
                if e.target() == e.current_target() && dismissable() {
                    dismiss();
                }
            }
        >
            <div class="he-dialog" role="dialog" aria-modal="true">
                <div class="he-dialog-header" class:he-hidden=move || !dismissable() && el.blank("title")>
                    <h2 class="he-dialog-title">{move || el.text("title")}</h2>
                    <button
                        class="hb-icon-button"
                        aria-label="Close"
                        class:he-hidden=move || !dismissable()
                        on:click=move |_| dismiss()
                    >
                        "✕"
                    </button>
                </div>
                <div class="he-dialog-body">{stack(el, "medium")}</div>
            </div>
        </div>
    }
}

// --- content -------------------------------------------------------------------

fn text(el: El) -> impl IntoView {
    move || {
        let t = el.text("text");
        match el.s("variant").as_deref() {
            Some("h1") => view! { <h1 class="he-h1">{t}</h1> }.into_any(),
            Some("h2") => view! { <h2 class="he-h2">{t}</h2> }.into_any(),
            Some("h3") => view! { <h3 class="he-h3">{t}</h3> }.into_any(),
            Some("caption") => view! { <p class="he-caption">{t}</p> }.into_any(),
            Some("code") => view! { <pre class="he-code"><code>{t}</code></pre> }.into_any(),
            _ => view! { <p class="he-body">{t}</p> }.into_any(),
        }
    }
}

fn notice(el: El) -> impl IntoView {
    let variant = move || el.s("variant").unwrap_or("info".into());
    view! {
        <div
            class="he-notice"
            role=move || if matches!(variant().as_str(), "warning" | "failure") { "alert" } else { "status" }
        >
            <span class="he-notice-icon" aria-hidden="true">
                {move || match variant().as_str() {
                    "success" => "✓",
                    "warning" => "!",
                    "failure" => "✕",
                    _ => "i",
                }}
            </span>
            <div class="he-notice-body">
                <strong class:he-hidden=move || el.blank("title")>{move || el.text("title")}</strong>
                <span class:he-hidden=move || el.blank("text")>{move || el.text("text")}</span>
            </div>
        </div>
    }
}

/// Only `data:` images are shown; other sources are not fetched and the alt
/// text shows instead, as in the other renderers.
fn image(el: El) -> impl IntoView {
    move || {
        let (src, alt) = (el.text("src"), el.text("alt"));
        if src.starts_with("data:") {
            view! { <img class="he-img" src=src alt=alt/> }.into_any()
        } else {
            view! { <span class="he-image-alt">{alt}</span> }.into_any()
        }
    }
}

fn progress(el: El) -> impl IntoView {
    view! {
        <div class="he-progress">
            <span class="he-label" class:he-hidden=move || el.blank("label")>{move || el.text("label")}</span>
            {move || {
                if el.s("variant").as_deref() == Some("spinner") {
                    view! { <div class="he-spinner" role="progressbar" aria-label=el.text("label")></div> }.into_any()
                } else if let Some(v) = el.n("value") {
                    view! { <progress max="1" prop:value=v.clamp(0.0, 1.0) aria-label=el.text("label")></progress> }
                        .into_any()
                } else {
                    view! { <progress aria-label=el.text("label")></progress> }.into_any()
                }
            }}
        </div>
    }
}

// --- controls ------------------------------------------------------------------

fn button(el: El) -> impl IntoView {
    view! {
        <button
            class="he-button"
            prop:disabled=move || el.disabled()
            on:click=move |_| el.input(None, Some("onPress"))
        >
            {move || el.text("label")}
        </button>
    }
}

/// The value of a number input: a number, or null if empty or invalid.
fn parse_number(text: &str) -> Value {
    let t = text.trim();
    if let Ok(i) = t.parse::<i64>() {
        return json!(i);
    }
    match t.parse::<f64>() {
        Ok(f) if f.is_finite() => json!(f),
        _ => Value::Null,
    }
}

fn number(v: f64) -> Value {
    if v.fract() == 0.0 && v.abs() < 9e15 {
        json!(v as i64)
    } else {
        json!(v)
    }
}

fn decimals(step: f64) -> usize {
    let s = format!("{step}");
    s.split_once('.').map_or(0, |(_, f)| f.len())
}

fn input(el: El) -> impl IntoView {
    let pending = StoredValue::new(false);
    let timer = StoredValue::new(None::<TimeoutHandle>);
    let fire = move || {
        if let Some(t) = timer.get_value() {
            t.clear();
            timer.set_value(None);
        }
        if pending.get_value() {
            pending.set_value(false);
            el.input(None, Some("onChange"));
        }
    };
    let edited = move |text: String| {
        let v = if el.s("variant").as_deref() == Some("number") {
            parse_number(&text)
        } else {
            Value::String(text)
        };
        el.input(Some(v), None);
        pending.set_value(true);
        if let Some(t) = timer.get_value() {
            t.clear();
        }
        let t = set_timeout_with_handle(
            move || {
                timer.set_value(None);
                fire();
            },
            CHANGE_IDLE,
        );
        timer.set_value(t.ok());
    };
    // What the field should show: the bound signal, unless the text already
    // means that (a number input keeps "1." while it is 1).
    let shown = move |current: Option<String>| -> String {
        if !el.has("bind") {
            return current.unwrap_or_default();
        }
        let bound = el.bound();
        if el.s("variant").as_deref() == Some("number") {
            let want = bound.as_ref().and_then(Value::as_f64);
            match current {
                Some(cur) if parse_number(&cur).as_f64() == want => cur,
                _ => want.map(|f| number(f).to_string()).unwrap_or_default(),
            }
        } else {
            bound
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_default()
        }
    };
    let multiline = Memo::new(move |_| el.s("variant").as_deref() == Some("multiline"));
    let kind = move || match el.s("variant").as_deref() {
        Some("password") => "password",
        Some("email") => "email",
        _ => "text",
    };
    let invalid = move || (!el.blank("error")).to_string();
    view! {
        <div class="he-input">
            <label class="he-label" for=el.dom_id() class:he-hidden=move || el.blank("label")>
                {move || el.text("label")}
            </label>
            {move || {
                if multiline.get() {
                    let node = NodeRef::<html::Textarea>::new();
                    view! {
                        <textarea
                            node_ref=node
                            id=el.dom_id()
                            rows="4"
                            placeholder=move || el.s("placeholder")
                            aria-invalid=invalid
                            prop:value=move || shown(node.get_untracked().map(|n| n.value()))
                            prop:disabled=move || el.disabled()
                            on:input:target=move |e| edited(e.target().value())
                            on:blur=move |_| fire()
                        ></textarea>
                    }
                        .into_any()
                } else {
                    let node = NodeRef::<html::Input>::new();
                    view! {
                        <input
                            node_ref=node
                            id=el.dom_id()
                            type=kind
                            inputmode=move || (el.s("variant").as_deref() == Some("number")).then_some("decimal")
                            placeholder=move || el.s("placeholder")
                            aria-invalid=invalid
                            prop:value=move || shown(node.get_untracked().map(|n| n.value()))
                            prop:disabled=move || el.disabled()
                            on:input:target=move |e| edited(e.target().value())
                            on:blur=move |_| fire()
                            on:keydown=move |e| {
                                if e.key() == "Enter" {
                                    fire();
                                    el.input(None, Some("onSubmit"));
                                }
                            }
                        />
                    }
                        .into_any()
                }
            }}
            <div class="he-error" class:he-hidden=move || el.blank("error")>{move || el.text("error")}</div>
        </div>
    }
}

fn checkbox(el: El) -> impl IntoView {
    let node = NodeRef::<html::Input>::new();
    let checked = move || match el.s("bind") {
        Some(_) => el.bound() == Some(Value::Bool(true)),
        None => node.get_untracked().is_some_and(|n| n.checked()),
    };
    view! {
        <label class="he-check">
            <input
                node_ref=node
                type="checkbox"
                role=move || (el.s("variant").as_deref() == Some("switch")).then_some("switch")
                prop:checked=checked
                prop:disabled=move || el.disabled()
                on:change:target=move |e| el.input(Some(Value::Bool(e.target().checked())), Some("onChange"))
            />
            <span>{move || el.text("label")}</span>
        </label>
    }
}

fn select(el: El) -> impl IntoView {
    let options = Memo::new(move |_| {
        el.attrs.with(|a| {
            a.get("options")
                .and_then(Value::as_array)
                .map(|l| {
                    l.iter()
                        .filter_map(|o| {
                            let o = o.as_object()?;
                            let v = o.get("value")?.as_str()?.to_string();
                            Some((v, o.get("label")?.as_str()?.to_string()))
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        })
    });
    // The choice of an unbound select.
    let local = RwSignal::new(None::<String>);
    let selected = Memo::new(move |_| match el.s("bind") {
        Some(_) => el.bound().and_then(|v| v.as_str().map(str::to_string)),
        None => local.get(),
    });
    let choose = move |v: String| {
        local.set(Some(v.clone()));
        el.input(Some(Value::String(v)), Some("onChange"));
    };
    let is = move |v: &str| selected.with(|s| s.as_deref() == Some(v));
    let name = el.dom_id();
    view! {
        <div class="he-select">
            <label class="he-label" for=el.dom_id() class:he-hidden=move || el.blank("label")>
                {move || el.text("label")}
            </label>
            {move || match el.s("variant").as_deref() {
                Some("radio") => {
                    let name = name.clone();
                    view! {
                        <div class="he-radios" role="radiogroup" id=el.dom_id()>
                            <For
                                each=move || options.get()
                                key=|o| o.clone()
                                children=move |(v, l)| {
                                    let v2 = v.clone();
                                    view! {
                                        <label class="he-check">
                                            <input
                                                type="radio"
                                                name=name.clone()
                                                prop:checked=move || is(&v)
                                                prop:disabled=move || el.disabled()
                                                on:change=move |_| choose(v2.clone())
                                            />
                                            <span>{l}</span>
                                        </label>
                                    }
                                }
                            />
                        </div>
                    }
                        .into_any()
                }
                Some("segmented") => {
                    view! {
                        <div class="he-segmented" role="radiogroup" id=el.dom_id()>
                            <For
                                each=move || options.get()
                                key=|o| o.clone()
                                children=move |(v, l)| {
                                    let v2 = v.clone();
                                    view! {
                                        <button
                                            role="radio"
                                            aria-checked=move || is(&v).to_string()
                                            prop:disabled=move || el.disabled()
                                            on:click=move |_| choose(v2.clone())
                                        >
                                            {l}
                                        </button>
                                    }
                                }
                            />
                        </div>
                    }
                        .into_any()
                }
                _ => {
                    view! {
                        <select
                            id=el.dom_id()
                            prop:disabled=move || el.disabled()
                            on:change:target=move |e| choose(e.target().value())
                        >
                            <option value="" disabled=true prop:selected=move || selected.with(Option::is_none)></option>
                            <For
                                each=move || options.get()
                                key=|o| o.clone()
                                children=move |(v, l)| {
                                    let v2 = v.clone();
                                    view! {
                                        <option value=v prop:selected=move || is(&v2)>
                                            {l}
                                        </option>
                                    }
                                }
                            />
                        </select>
                    }
                        .into_any()
                }
            }}
        </div>
    }
}

fn slider(el: El) -> impl IntoView {
    let min = move || el.n("min").unwrap_or(0.0);
    let max = move || el.n("max").unwrap_or(100.0).max(min());
    let step = move || el.n("step").filter(|s| *s > 0.0).unwrap_or(1.0);
    // The value of an unbound slider.
    let local = RwSignal::new(None::<f64>);
    let pending = StoredValue::new(false);
    let value = move || {
        let v = match el.s("bind") {
            Some(_) => el.bound().and_then(|v| v.as_f64()),
            None => local.get(),
        };
        v.unwrap_or(min()).clamp(min(), max())
    };
    view! {
        <div class="he-slider">
            <div class="he-slider-head">
                <label class="he-label" for=el.dom_id()>{move || el.text("label")}</label>
                <output for=el.dom_id()>{move || format!("{:.*}", decimals(step()), value())}</output>
            </div>
            <input
                id=el.dom_id()
                type="range"
                min=move || min().to_string()
                max=move || max().to_string()
                step=move || step().to_string()
                prop:value=move || value().to_string()
                prop:disabled=move || el.disabled()
                on:input:target=move |e| {
                    if let Ok(v) = e.target().value().parse::<f64>() {
                        local.set(Some(v));
                        el.input(Some(number(v)), None);
                        pending.set_value(true);
                    }
                }
                on:change=move |_| {
                    if pending.get_value() {
                        pending.set_value(false);
                        el.input(None, Some("onChange"));
                    }
                }
            />
        </div>
    }
}
