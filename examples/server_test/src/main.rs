#[cfg(feature = "ssr")]
#[tokio::main]
async fn main() {
    use axum::Router;
    use leptos::config::get_configuration;
    use leptos::prelude::*;
    use leptos_axum::{generate_route_list, LeptosRoutes};
    use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

    use server_test::app::{shell, App};
    use server_test::server::{
        build_socketio_layer, spawn_log_pump, BroadcastLayer, LogBus,
    };

    let bus = LogBus::new();

    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new("info,server_test=debug,socket_io_client=debug,socketioxide=warn,engineioxide=warn,tower_http=warn")
    });

    tracing_subscriber::registry()
        .with(env_filter)
        .with(tracing_subscriber::fmt::layer().with_target(true))
        .with(BroadcastLayer::new(bus.sender()))
        .init();

    tracing::info!("server_test demo 启动中...");

    let conf = get_configuration(None).expect("load leptos config");
    let leptos_options = conf.leptos_options;
    let addr = leptos_options.site_addr;

    let (sio_layer, io) = build_socketio_layer();

    // 把 tracing 事件通过 socketioxide 广播给浏览器
    spawn_log_pump(bus, io);

    let routes = generate_route_list(App);

    let app = Router::new()
        .leptos_routes(&leptos_options, routes, {
            let leptos_options = leptos_options.clone();
            move || shell(leptos_options.clone())
        })
        .fallback(leptos_axum::file_and_error_handler::<LeptosOptions, _>(shell))
        .with_state(leptos_options)
        .layer(sio_layer);

    tracing::info!("监听 http://{}", addr);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("bind tcp listener");
    axum::serve(listener, app.into_make_service())
        .await
        .expect("axum serve");
}

#[cfg(not(feature = "ssr"))]
fn main() {}
