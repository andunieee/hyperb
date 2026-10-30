#[cfg(feature = "ssr")]
#[tokio::main]
async fn main() {
    use axum::Router;
    use hyperb_web::{App, shell};
    use leptos_axum::{LeptosRoutes, file_and_error_handler, generate_route_list};

    // Read [package.metadata.leptos] directly so a plain `cargo run` works;
    // under cargo-leptos its LEPTOS_* env vars override these values.
    let conf =
        leptos::config::get_configuration(Some(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml")))
            .unwrap();
    let addr = conf.leptos_options.site_addr;
    let routes = generate_route_list(App);

    let app = Router::new()
        .leptos_routes(&conf.leptos_options, routes, {
            let opts = conf.leptos_options.clone();
            move || shell(opts.clone())
        })
        .fallback(file_and_error_handler(shell))
        .with_state(conf.leptos_options);

    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    println!("hyperb-web listening on http://{addr}");
    axum::serve(listener, app.into_make_service())
        .await
        .unwrap();
}

#[cfg(not(feature = "ssr"))]
fn main() {}
