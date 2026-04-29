use tokio::sync::broadcast;
use tracing::field::{Field, Visit};
use tracing_subscriber::layer::Context;
use tracing_subscriber::Layer;

const LOG_CHANNEL_CAPACITY: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogKind {
    Server,
    Client,
    Other,
}

impl LogKind {
    /// socket.io 里推送日志时使用的事件名。
    pub fn event_name(self) -> &'static str {
        match self {
            LogKind::Server => "log:server",
            LogKind::Client => "log:client",
            LogKind::Other => "log:other",
        }
    }
}

#[derive(Clone, Debug)]
pub struct LogLine {
    pub kind: LogKind,
    pub text: String,
}

#[derive(Clone)]
pub struct LogBus {
    tx: broadcast::Sender<LogLine>,
}

impl LogBus {
    pub fn new() -> Self {
        let (tx, _rx) = broadcast::channel(LOG_CHANNEL_CAPACITY);
        Self { tx }
    }

    pub fn sender(&self) -> broadcast::Sender<LogLine> {
        self.tx.clone()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<LogLine> {
        self.tx.subscribe()
    }
}

/// 捕捉 `tracing` 事件并分类后写入广播通道的 Layer。
///
/// 分类策略：按事件的 `target`（= module path）前缀区分
/// - `server_test::server::socketio_server` 前缀 → `Server`
/// - `server_test::server::socketio_client` 前缀 → `Client`
/// - 其它（含 socketioxide / reqwest / hyper 等）→ `Other`
pub struct BroadcastLayer {
    tx: broadcast::Sender<LogLine>,
}

impl BroadcastLayer {
    pub fn new(tx: broadcast::Sender<LogLine>) -> Self {
        Self { tx }
    }
}

struct MsgVisitor {
    message: Option<String>,
    fields: Vec<(String, String)>,
}

impl<'a> Visit for MsgVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = Some(value.to_string());
        } else {
            self.fields.push((field.name().to_string(), value.to_string()));
        }
    }
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = Some(format!("{:?}", value));
        } else {
            self.fields.push((field.name().to_string(), format!("{:?}", value)));
        }
    }
}

fn classify(target: &str) -> LogKind {
    if target.starts_with("server_test::server::socketio_server") {
        LogKind::Server
    } else if target.starts_with("server_test::server::socketio_client") {
        LogKind::Client
    } else {
        LogKind::Other
    }
}

impl<S> Layer<S> for BroadcastLayer
where
    S: tracing::Subscriber,
{
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let meta = event.metadata();
        let kind = classify(meta.target());
        
        let mut visitor = MsgVisitor {
            message: None,
            fields: Vec::new(),
        };
        event.record(&mut visitor);

        let mut text = String::new();
        // 1. 打印日志级别和 Target
        text.push_str(&format!("[{}] ", meta.level()));
        
        // 2. 打印主消息 (如果有)
        if let Some(msg) = visitor.message {
            text.push_str(&msg);
        }

        // 3. 如果有附加字段，换行缩进格式化显示
        if !visitor.fields.is_empty() {
            text.push_str("\n  ↳ ");
            let fields_str = visitor.fields
                .into_iter()
                .map(|(k, v)| format!("{}: {}", k, v))
                .collect::<Vec<_>>()
                .join(", ");
            text.push_str(&fields_str);
        }

        let _ = self.tx.send(LogLine { kind, text });
    }
}
