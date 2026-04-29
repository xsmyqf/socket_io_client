//! client_test 入口文件
//!
//! 负责初始化日志并启动 Socket.IO 客户端。

mod socketio_client;

use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

#[tokio::main]
async fn main() {
    // 初始化日志
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new("info,client_test=info,socket_io_client=debug,socketioxide=debug,engineioxide=trace,tower_http=debug")
    });

    tracing_subscriber::registry()
        .with(env_filter)
        .with(tracing_subscriber::fmt::layer().with_target(true))
        .init();

    info!("独立 socket_io_client 启动中...");

    // 启动客户端
    socketio_client::start_client().await;
}
