pub mod logs;
pub mod socketio_server;

use socketioxide::SocketIo;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

pub use logs::{BroadcastLayer, LogBus, LogKind, LogLine};
pub use socketio_server::build_socketio_layer;

/// 订阅 LogBus，把每条日志按 `server` / `client` 分类通过 socketioxide 广播给所有浏览器连接。
pub fn spawn_log_pump(bus: LogBus, io: SocketIo) {
    tokio::spawn(async move {
        let rx = bus.subscribe();
        let mut stream = BroadcastStream::new(rx);
        while let Some(msg) = stream.next().await {
            if let Ok(line) = msg {
                let event = line.kind.event_name();
                let _ = io.emit(event, &line.text).await;
            }
        }
    });
}
