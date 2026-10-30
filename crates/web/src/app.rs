//! The web renderer: a hypereffect browser in the page.
//!
//! The server renders the page shell; once the wasm bundle hydrates it, the
//! browser ([`browser`]) connects to hypereffect servers from the page itself.

// The view tree is one deeply nested generic type; the default depth is too
// small for the release build.
#![recursion_limit = "256"]

use leptos::prelude::*;
use leptos_meta::{Link, MetaTags, Stylesheet, Title, provide_meta_context};
use leptos_router::{
    StaticSegment,
    components::{Route, Router, Routes},
};

#[cfg(feature = "hydrate")]
mod browser;
#[cfg(feature = "hydrate")]
mod render;
#[cfg(feature = "hydrate")]
mod store;

/// Full HTML document rendered by the server; the `<head>` is where
/// leptos_meta injects `<Title>` and friends.
pub fn shell(options: LeptosOptions) -> impl IntoView {
    view! {
        <!DOCTYPE html>
        <html lang="en">
            <head>
                <meta charset="utf-8"/>
                <meta name="viewport" content="width=device-width, initial-scale=1"/>
                <AutoReload options=options.clone()/>
                <HydrationScripts options/>
                <MetaTags/>
            </head>
            <body>
                <App/>
            </body>
        </html>
    }
}

#[cfg(feature = "hydrate")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn hydrate() {
    console_error_panic_hook::set_once();
    leptos::mount::hydrate_body(App);
}

#[component]
pub fn App() -> impl IntoView {
    provide_meta_context();
    view! {
        <Stylesheet id="leptos" href="/pkg/hyperb-web.css"/>
        <Title text="hyperb"/>
        <Link rel="icon" type_="image/svg+xml" href="/icon.svg"/>
        <Router>
            // The page is served from "/" and from a sub-path on Pages (and
            // as 404.html for any other), so every path shows the browser;
            // a fallback other than Home would not match the server's HTML.
            <Routes fallback=Home>
                <Route path=StaticSegment("") view=Home/>
            </Routes>
        </Router>
    }
}

/// The browser, which only runs in the page: the server renders a
/// placeholder, and the browser replaces it once the page has hydrated.
#[component]
fn Home() -> impl IntoView {
    let mounted = RwSignal::new(false);
    Effect::new(move |_| mounted.set(true));
    view! {
        <Show when=move || mounted.get() fallback=|| view! { <p class="hb-loading">"Loading…"</p> }>
            {client()}
        </Show>
    }
}

#[cfg(feature = "hydrate")]
fn client() -> AnyView {
    view! { <browser::Browser/> }.into_any()
}

#[cfg(not(feature = "hydrate"))]
fn client() -> AnyView {
    ().into_any()
}
