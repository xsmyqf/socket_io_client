//! 底层 Socket.IO 协议传输客户端实现。
//! Underlying Socket.IO protocol transport client implementation.
//!
//! 负责与 Socket.IO 服务端建立连接（支持 HTTP 长轮询及 WebSocket 升级）、维护心跳（Ping/Pong）、
//! 解析和发送 Engine.IO 与 Socket.IO 报文，并支持断线重连及多命名空间（Namespace）管理。
//! Responsible for establishing connections with the Socket.IO server (supports HTTP long-polling and WebSocket upgrades),
//! maintaining heartbeats (Ping/Pong), parsing and sending Engine.IO and Socket.IO packets, 
//! and supporting auto-reconnection and multi-namespace management.

use base64::{Engine as _, engine::general_purpose};
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{
        Arc as StdArc, RwLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::task::JoinHandle;
use tokio::{
    net::TcpStream,
    sync::{Mutex, Notify, mpsc, oneshot},
    time::sleep,
};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async, tungstenite::protocol::Message,
};

use super::codec::{
    encode_binary_header, encode_with_namespace, encode_with_namespace_with_id,
    parse_binary_header, parse_ns_payload, parse_ns_payload_with_id,
};

// ---------------------------------------------------------
// 公共类型
// ---------------------------------------------------------

/// 传递给事件回调的完整上下文。
/// The complete context passed to the event callback.
#[derive(Clone, Debug)]
pub struct IncomingEvent {
    /// 事件负载（数组，包含 event 之后的参数）或由系统事件注入的特殊值。
    /// The event payload (array containing parameters after the event) or a special value injected by system events.
    pub data: Value,
    /// 服务端期望客户端回复 ACK 时的 id；系统事件和无 ack 的普通事件为 `None`。
    /// The id when the server expects the client to reply with an ACK; `None` for system events and normal events without ack.
    pub ack_id: Option<u64>,
    /// 事件所属的命名空间。
    /// The namespace to which the event belongs.
    pub namespace: String,
}

/// 异步事件处理函数的类型别名。
/// Type alias for asynchronous event handler functions.
pub type AsyncEventCallback =
    StdArc<dyn Fn(IncomingEvent) -> futures::future::BoxFuture<'static, ()> + Send + Sync>;

type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

// 系统事件常量（以下划线开头，避免与真实业务事件冲突）。
// System event constants (starting with underscores to avoid conflicts with real business events).

/// 内部常量：连接成功事件名
/// Internal constant: Event name for successful connection.
pub const SYS_CONNECT: &str = "__connect__";
/// 内部常量：断开连接事件名
/// Internal constant: Event name for disconnection.
pub const SYS_DISCONNECT: &str = "__disconnect__";
/// 内部常量：错误事件名
/// Internal constant: Event name for errors.
pub const SYS_ERROR: &str = "__error__";
/// 内部常量：连接错误事件名
/// Internal constant: Event name for connection errors.
pub const SYS_CONNECT_ERROR: &str = "__connect_error__";
/// 内部常量：连接显式关闭事件名
/// Internal constant: Event name for explicit connection closure.
pub const SYS_CLOSED: &str = "__closed__";
/// 内部常量：尝试重连事件名
/// Internal constant: Event name for a reconnection attempt.
pub const SYS_RECONNECT_ATTEMPT: &str = "__reconnect_attempt__";
/// 内部常量：重连成功事件名
/// Internal constant: Event name for successful reconnection.
pub const SYS_RECONNECT: &str = "__reconnect__";

/// 底层传输通道，用于观察当前使用的是 polling 还是 websocket。
/// The underlying transport channel, used to observe whether polling or websocket is currently used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TransportMode {
    Polling,
    WebSocket,
}

/// 通过写通道下发的 Engine.IO 原始报文。
/// The raw Engine.IO packet sent through the write channel.
#[derive(Debug)]
pub enum OutgoingPacket {
    /// 文本报文（已经是 Engine.IO 层的完整文本，例如 "42[...]"）。
    /// Text packet (already a complete text at the Engine.IO layer, e.g., "42[...]").
    Text(String),
    /// 二进制附件（通常紧跟在 `45`/`46` 头后）。
    /// Binary attachment (usually follows a `45`/`46` header).
    Binary(Vec<u8>),
}

/// 读通道里收到的 Engine.IO 原始帧。
#[derive(Debug)]
enum IncomingFrame {
    Text(String),
    Binary(Vec<u8>),
}

// ---------------------------------------------------------
// 握手响应
// ---------------------------------------------------------

#[derive(Deserialize, Debug, Clone)]
#[allow(non_snake_case, dead_code)]
struct HandshakeResponse {
    sid: String,
    upgrades: Vec<String>,
    pingInterval: u64,
    pingTimeout: u64,
    #[serde(default)]
    maxPayload: u64,
}

// ---------------------------------------------------------
// 每个 namespace 的状态
// ---------------------------------------------------------

#[derive(Default)]
struct NsState {
    sid: Option<String>,
    event_handlers: HashMap<String, Vec<AsyncEventCallback>>,
    ack_waiters: HashMap<u64, oneshot::Sender<Value>>,
    auth: Option<Value>,
    connected: bool,
}

// ---------------------------------------------------------
// 等待组装的二进制事件
// ---------------------------------------------------------

struct PendingBinary {
    ns: String,
    is_ack: bool,
    ack_id: Option<u64>,
    template: Value,
    num_expected: usize,
    attachments: Vec<Vec<u8>>,
}

// ---------------------------------------------------------
// 客户端
// ---------------------------------------------------------

/// 底层 Socket.IO 客户端。
/// Underlying Socket.IO client.
///
/// 直接操作 `SocketIOClient` 允许精细控制重连、事件注册及命名空间。
/// Direct manipulation of `SocketIOClient` allows fine-grained control over reconnection, event registration, and namespaces.
/// 推荐在业务层使用 [`ClientApp`](crate::ClientApp) 来管理 `SocketIOClient` 实例。
/// It is recommended to use [`ClientApp`](crate::ClientApp) at the business layer to manage `SocketIOClient` instances.
#[derive(Clone)]
pub struct SocketIOClient {
    /// 服务端基础地址，例如 `http://127.0.0.1:5000`
    /// Base URL of the server, e.g., `http://127.0.0.1:5000`
    pub base_url: String,
    /// 主命名空间，默认为 `/`
    /// Primary namespace, defaults to `/`
    pub namespace: String,

    outgoing_tx: StdArc<RwLock<Option<mpsc::UnboundedSender<OutgoingPacket>>>>,
    ns_states: StdArc<RwLock<HashMap<String, NsState>>>,
    ack_id_counter: StdArc<AtomicU64>,
    transport_mode: StdArc<RwLock<TransportMode>>,
    http: reqwest::Client,

    reconnect_interval: Duration,
    retry_count: usize,
    pong_timeout: Duration,
    /// 是否在初始 WS 失败后允许留在 polling 模式（官方叫 `tryAllTransports`）。
    /// Whether to allow staying in polling mode after initial WS failure (officially called `tryAllTransports`).
    try_polling_fallback: bool,
    /// 是否允许 polling 成功后尝试升级到 WS。
    /// Whether to allow upgrading to WS after a successful polling connection.
    allow_upgrade: bool,

    disconnect_notify: StdArc<Notify>,
    ping_task: StdArc<Mutex<Option<JoinHandle<()>>>>,
    event_task: StdArc<Mutex<Option<JoinHandle<()>>>>,
    writer_task: StdArc<Mutex<Option<JoinHandle<()>>>>,
    reader_task: StdArc<Mutex<Option<JoinHandle<()>>>>,
}

impl SocketIOClient {
    /// 使用给定的 `base_url` 创建一个新的 `SocketIOClient` 实例。
    /// Creates a new `SocketIOClient` instance with the given `base_url`.
    /// 默认支持长轮询升级到 WebSocket，并在 `/` 命名空间初始化。
    /// Supports HTTP long-polling upgrading to WebSocket by default, and initializes at the `/` namespace.
    pub fn new(base_url: &str) -> Self {
        let mut ns_states = HashMap::new();
        ns_states.insert("/".to_string(), NsState::default());

        Self {
            base_url: base_url.to_string(),
            namespace: "/".to_string(),
            outgoing_tx: StdArc::new(RwLock::new(None)),
            ns_states: StdArc::new(RwLock::new(ns_states)),
            ack_id_counter: StdArc::new(AtomicU64::new(1)),
            transport_mode: StdArc::new(RwLock::new(TransportMode::Polling)),
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(120))
                .build()
                .unwrap_or_default(),
            reconnect_interval: Duration::from_secs(5),
            retry_count: 0,
            pong_timeout: Duration::from_secs(30),
            try_polling_fallback: true,
            allow_upgrade: true,
            disconnect_notify: StdArc::new(Notify::new()),
            ping_task: StdArc::new(Mutex::new(None)),
            event_task: StdArc::new(Mutex::new(None)),
            writer_task: StdArc::new(Mutex::new(None)),
            reader_task: StdArc::new(Mutex::new(None)),
        }
    }

    // -------------------------
    // 配置
    // -------------------------

    /// 设置重连间隔时间（秒）。
    /// Sets the reconnection interval in seconds.
    pub fn set_reconnect_interval(&mut self, secs: u64) {
        self.reconnect_interval = Duration::from_secs(secs);
    }

    /// 设置重试重连的次数。设为 0 表示无限重试。
    /// Sets the number of reconnection retries. Set to 0 for infinite retries.
    pub fn set_retry_count(&mut self, count: usize) {
        self.retry_count = count;
    }

    /// 设置超时时间（秒），若在此时限内未收到服务端的心跳响应，则触发断开逻辑。
    /// Sets the timeout in seconds. If no heartbeat response is received within this time, the connection is disconnected.
    pub fn set_pong_timeout(&mut self, secs: u64) {
        self.pong_timeout = Duration::from_secs(secs);
    }

    /// 切换当前操作的默认命名空间。
    /// Switches the default namespace for current operations.
    pub fn set_namespace(&mut self, ns: &str) {
        let ns = normalize_ns(ns);
        self.ensure_ns(&ns);
        self.namespace = ns;
    }

    /// 是否在尝试建立 WebSocket 失败后允许降级回长轮询（Polling）。
    /// Whether to allow falling back to Polling if WebSocket establishment fails.
    pub fn set_try_polling_fallback(&mut self, v: bool) {
        self.try_polling_fallback = v;
    }

    /// 是否允许从 HTTP 长轮询自动升级到 WebSocket。
    /// Whether to allow automatic upgrading from HTTP long-polling to WebSocket.
    pub fn set_allow_upgrade(&mut self, v: bool) {
        self.allow_upgrade = v;
    }

    /// 返回当前实际使用的传输协议模式（Polling / WebSocket）。
    /// Returns the transport protocol mode currently in use (Polling / WebSocket).
    pub fn transport(&self) -> TransportMode {
        self.transport_mode.read().unwrap().clone()
    }

    /// 设置连接特定命名空间时的认证信息（Auth Payload）。
    /// Sets the authentication information (Auth Payload) when connecting to a specific namespace.
    pub fn set_auth(&mut self, ns: &str, auth: Value) {
        let ns = normalize_ns(ns);
        self.ensure_ns(&ns);
        let mut guard = self.ns_states.write().unwrap();
        if let Some(s) = guard.get_mut(&ns) {
            s.auth = Some(auth);
        }
    }

    /// 清除特定命名空间的认证信息。
    /// Clears the authentication information of a specific namespace.
    pub fn clear_auth(&mut self, ns: &str) {
        let ns = normalize_ns(ns);
        let mut guard = self.ns_states.write().unwrap();
        if let Some(s) = guard.get_mut(&ns) {
            s.auth = None;
        }
    }

    /// 预先注册一个命名空间。只有被注册的命名空间才会在握手时被尝试连接。
    /// Pre-registers a namespace. Only registered namespaces will be attempted to connect during handshake.
    pub fn register_namespace(&mut self, ns: &str) {
        self.ensure_ns(&normalize_ns(ns));
    }

    fn ensure_ns(&self, ns: &str) {
        let mut guard = self.ns_states.write().unwrap();
        guard.entry(ns.to_string()).or_default();
    }

    // -------------------------
    // 事件注册
    // -------------------------

    /// 监听当前命名空间下的指定事件。
    /// Listens to the specified event under the current namespace.
    pub fn on<F>(&mut self, event: &str, cb: F)
    where
        F: Fn(IncomingEvent) -> futures::future::BoxFuture<'static, ()> + Send + Sync + 'static,
    {
        self.on_ns(&self.namespace.clone(), event, cb);
    }

    /// 监听指定命名空间下的指定事件。
    /// Listens to the specified event under the specified namespace.
    pub fn on_ns<F>(&mut self, ns: &str, event: &str, cb: F)
    where
        F: Fn(IncomingEvent) -> futures::future::BoxFuture<'static, ()> + Send + Sync + 'static,
    {
        let ns = normalize_ns(ns);
        self.ensure_ns(&ns);
        let mut map = self.ns_states.write().unwrap();
        let st = map.entry(ns).or_default();
        let entry = st.event_handlers.entry(event.to_string()).or_default();
        entry.push(StdArc::new(cb));
    }

    // -------------------------
    // HTTP 握手（polling 方式，拿 sid 与 upgrades）
    // -------------------------
    async fn handshake(&self) -> Result<HandshakeResponse, Box<dyn std::error::Error + Send + Sync>> {
        let polling_url = format!("{}/socket.io/?EIO=4&transport=polling", self.base_url);
        let resp = self.http.get(&polling_url).send().await?;
        if !resp.status().is_success() {
            return Err(format!("handshake failed: HTTP {}", resp.status()).into());
        }
        let text = resp.text().await?;

        let first = text.split('\u{1e}').next().unwrap_or("");
        let json_part = first
            .strip_prefix('0')
            .ok_or("handshake: expected Engine.IO open packet prefix '0'")?;
        Ok(serde_json::from_str(json_part)?)
    }

    // -------------------------
    // 建立连接
    // -------------------------
    
    /// 发起一次连接尝试，进行握手并启动收发循环及心跳任务。
    /// Initiates a connection attempt, performs the handshake, and starts the send/receive loop and heartbeat task.
    ///
    /// 建议通过 [`run_with_reconnect`](Self::run_with_reconnect) 实现带重连的持久连接。
    /// It is recommended to use [`run_with_reconnect`](Self::run_with_reconnect) to achieve a persistent connection with auto-reconnection.
    pub async fn connect(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let hs = self.handshake().await?;

        // 建立两个通道：
        // incoming_tx 由 reader 填；event_processor 从 incoming_rx 读。
        // outgoing_tx 由外部 emit/pong 填；writer 从 outgoing_rx 读。
        let (incoming_tx, incoming_rx) = mpsc::unbounded_channel::<IncomingFrame>();
        let (outgoing_tx, outgoing_rx) = mpsc::unbounded_channel::<OutgoingPacket>();
        let mut outgoing_rx_opt = Some(outgoing_rx);

        // 先尝试 WebSocket（若 upgrades 里包含且 allow_upgrade 为真）。
        let ws_supported = hs.upgrades.iter().any(|u| u == "websocket");
        let mut chosen_mode = TransportMode::Polling;
        let mut writer_handle: Option<JoinHandle<()>> = None;
        let mut reader_handle: Option<JoinHandle<()>> = None;

        if self.allow_upgrade && ws_supported {
            match self.try_open_ws(&hs.sid).await {
                Ok((sink, stream)) => {
                    // 升级成功
                    chosen_mode = TransportMode::WebSocket;
                    let tx_clone = incoming_tx.clone();
                    reader_handle = Some(tokio::spawn(ws_reader_loop(stream, tx_clone)));
                    let rx = outgoing_rx_opt.take().unwrap();
                    writer_handle = Some(tokio::spawn(ws_writer_loop(sink, rx)));
                }
                Err(e) => {
                    if !self.try_polling_fallback {
                        return Err(e);
                    }
                    eprintln!(
                        "[socket_io_client] websocket upgrade failed ({e}); falling back to polling"
                    );
                }
            }
        }

        if chosen_mode == TransportMode::Polling {
            let http = self.http.clone();
            let base = self.base_url.clone();
            let sid = hs.sid.clone();
            reader_handle = Some(tokio::spawn(polling_reader_loop(
                http.clone(),
                base.clone(),
                sid.clone(),
                incoming_tx.clone(),
            )));
            let rx = outgoing_rx_opt.take().unwrap();
            writer_handle = Some(tokio::spawn(polling_writer_loop(
                http, base, sid, rx,
            )));
        }

        *self.transport_mode.write().unwrap() = chosen_mode.clone();

        // 发送所有已登记 ns 的 CONNECT 报文
        let ns_list: Vec<(String, Option<Value>)> = {
            let guard = self.ns_states.read().unwrap();
            guard
                .iter()
                .map(|(k, v)| (k.clone(), v.auth.clone()))
                .collect()
        };
        for (ns, auth) in ns_list {
            let _ = outgoing_tx.send(OutgoingPacket::Text(format_connect_packet(&ns, auth)));
        }

        // 保存 outgoing_tx，供 emit 使用
        *self.outgoing_tx.write().unwrap() = Some(outgoing_tx.clone());

        // ping 看门狗 —— 用协商好的 (pingInterval + pingTimeout) 作为上限
        let last_ping = StdArc::new(Mutex::new(Instant::now()));
        let last_ping_watch = last_ping.clone();
        let disconnect_notify = self.disconnect_notify.clone();

        let negotiated = if hs.pingInterval > 0 && hs.pingTimeout > 0 {
            Duration::from_millis(hs.pingInterval + hs.pingTimeout)
        } else {
            Duration::from_secs(0)
        };
        let pong_timeout = if negotiated.as_millis() > 0 {
            std::cmp::max(self.pong_timeout, negotiated)
        } else {
            self.pong_timeout
        };

        let ns_states_for_watchdog = self.ns_states.clone();
        let reader_for_wd = self.reader_task.clone();
        let writer_for_wd = self.writer_task.clone();
        let event_for_wd = self.event_task.clone();
        let ping_handle = tokio::spawn(async move {
            loop {
                sleep(Duration::from_secs(2)).await;
                let elapsed = last_ping_watch.lock().await.elapsed();
                if elapsed > pong_timeout {
                    broadcast_system_event(
                        &ns_states_for_watchdog,
                        SYS_ERROR,
                        Value::String(format!("ping timeout: {elapsed:?}")),
                    )
                    .await;
                    broadcast_system_event(&ns_states_for_watchdog, SYS_DISCONNECT, Value::Null)
                        .await;
                    if let Some(h) = reader_for_wd.lock().await.take() {
                        h.abort();
                    }
                    if let Some(h) = writer_for_wd.lock().await.take() {
                        h.abort();
                    }
                    if let Some(h) = event_for_wd.lock().await.take() {
                        h.abort();
                    }
                    disconnect_notify.notify_one();
                    break;
                }
            }
        });

        // 事件处理循环
        let ns_states_loop = self.ns_states.clone();
        let disconnect_notify_loop = self.disconnect_notify.clone();
        let outgoing_for_loop = outgoing_tx.clone();
        let outgoing_stored = self.outgoing_tx.clone();
        let ping_task_handle_arc2 = self.ping_task.clone();
        let reader_handle_arc = self.reader_task.clone();
        let writer_handle_arc = self.writer_task.clone();
        let last_ping_for_loop = last_ping;

        let event_handle = tokio::spawn(async move {
            run_event_processor(
                incoming_rx,
                ns_states_loop.clone(),
                outgoing_for_loop,
                last_ping_for_loop,
            )
            .await;

            // 循环结束后清理
            if let Some(h) = ping_task_handle_arc2.lock().await.take() {
                h.abort();
            }
            if let Some(h) = reader_handle_arc.lock().await.take() {
                h.abort();
            }
            if let Some(h) = writer_handle_arc.lock().await.take() {
                h.abort();
            }
            {
                let mut guard = outgoing_stored.write().unwrap();
                *guard = None;
            }
            {
                let mut guard = ns_states_loop.write().unwrap();
                for (_, st) in guard.iter_mut() {
                    st.ack_waiters.clear();
                    st.connected = false;
                    st.sid = None;
                }
            }
            disconnect_notify_loop.notify_one();
        });

        {
            let mut h = self.ping_task.lock().await;
            *h = Some(ping_handle);
        }
        {
            let mut h = self.event_task.lock().await;
            *h = Some(event_handle);
        }
        if let Some(h) = reader_handle {
            *self.reader_task.lock().await = Some(h);
        }
        if let Some(h) = writer_handle {
            *self.writer_task.lock().await = Some(h);
        }

        Ok(())
    }

    /// 尝试打开 WebSocket 并完成 `2probe / 3probe / 5` 升级。
    async fn try_open_ws(
        &self,
        sid: &str,
    ) -> Result<
        (
            futures::stream::SplitSink<WsStream, Message>,
            futures::stream::SplitStream<WsStream>,
        ),
        Box<dyn std::error::Error + Send + Sync>,
    > {
        let ws_base = if self.base_url.starts_with("https://") {
            self.base_url.replacen("https://", "wss://", 1)
        } else {
            self.base_url.replacen("http://", "ws://", 1)
        };
        let ws_url = format!(
            "{ws_base}/socket.io/?EIO=4&transport=websocket&sid={sid}"
        );

        let (ws_stream, _) = connect_async(&ws_url).await?;
        let (mut sink, mut stream) = ws_stream.split();

        // 升级握手：2probe → 3probe → 5
        sink.send(Message::Text("2probe".into())).await?;
        let probe_timeout = Duration::from_secs(5);
        let probe_ok: Result<bool, Box<dyn std::error::Error + Send + Sync>> =
            tokio::time::timeout(probe_timeout, async {
                loop {
                    match stream.next().await {
                        Some(Ok(Message::Text(t))) => {
                            let s: &str = t.as_ref();
                            if s == "3probe" {
                                return Ok::<_, Box<dyn std::error::Error + Send + Sync>>(true);
                            }
                            if s.starts_with('0') {
                                // 服务端已直接以 websocket-only 模式打开
                                return Ok(true);
                            }
                        }
                        Some(Ok(_)) => continue,
                        Some(Err(e)) => return Err(e.into()),
                        None => return Err("websocket closed during probe".into()),
                    }
                }
            })
            .await
            .map_err(|_| "websocket probe timeout".to_string())?;

        if !probe_ok? {
            return Err("websocket probe failed".into());
        }
        sink.send(Message::Text("5".into())).await?;
        Ok((sink, stream))
    }

    // -------------------------
    // 自动重连循环
    // -------------------------
    
    /// 阻塞当前任务并不断运行客户端连接。
    /// Blocks the current task and continuously runs the client connection.
    /// 
    /// 当连接断开时，会按照配置的重连间隔（`reconnect_interval`）和重试次数（`retry_count`）尝试恢复连接。
    /// When the connection is disconnected, it attempts to recover the connection according to the configured reconnection interval (`reconnect_interval`) and retry count (`retry_count`).
    pub async fn run_with_reconnect(&mut self) {
        let mut is_reconnect = false;
        let mut attempts = 0;
        loop {
            if is_reconnect {
                attempts += 1;
                broadcast_system_event(&self.ns_states, SYS_RECONNECT_ATTEMPT, Value::Number(attempts.into())).await;
            }
            let res = self.connect().await;
            if res.is_ok() {
                if is_reconnect {
                    broadcast_system_event(&self.ns_states, SYS_RECONNECT, Value::Number(attempts.into())).await;
                }
                is_reconnect = true;
                attempts = 0;
                self.disconnect_notify.notified().await;
            } else {
                if let Err(e) = res {
                    eprintln!("SocketIO connect failed: {e:?}");
                }
                is_reconnect = true;
            }

            if self.retry_count > 0 && attempts >= self.retry_count {
                if let Some(h) = self.ping_task.lock().await.take() {
                    h.abort();
                }
                if let Some(h) = self.event_task.lock().await.take() {
                    h.abort();
                }
                if let Some(h) = self.reader_task.lock().await.take() {
                    h.abort();
                }
                if let Some(h) = self.writer_task.lock().await.take() {
                    h.abort();
                }
                broadcast_system_event(&self.ns_states, SYS_CLOSED, Value::Null).await;
                break;
            }

            sleep(self.reconnect_interval).await;
        }
    }

    // -------------------------
    // 发送 Socket.IO 报文
    // -------------------------

    /// 往默认命名空间发送普通的 JSON 数据事件。
    /// Emits a normal JSON data event to the default namespace.
    pub async fn emit<T: Serialize>(
        &self,
        event: &str,
        data: T,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.emit_ns(&self.namespace, event, data).await
    }

    /// 往指定的命名空间发送 JSON 数据事件。
    /// Emits a JSON data event to the specified namespace.
    pub async fn emit_ns<T: Serialize>(
        &self,
        ns: &str,
        event: &str,
        data: T,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let ns = normalize_ns(ns);
        let payload = json!([event, data]).to_string();
        let msg = encode_with_namespace(&ns, "42", &payload);
        self.send_outgoing(OutgoingPacket::Text(msg))
    }

    /// 往默认命名空间发送多参数事件。
    /// Emits a multi-argument event to the default namespace.
    pub async fn emit_args(
        &self,
        event: &str,
        args: Vec<Value>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.emit_args_ns(&self.namespace, event, args).await
    }

    /// 往指定命名空间发送多参数事件。
    /// Emits a multi-argument event to the specified namespace.
    pub async fn emit_args_ns(
        &self,
        ns: &str,
        event: &str,
        args: Vec<Value>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let ns = normalize_ns(ns);
        let mut data: Vec<Value> = Vec::with_capacity(1 + args.len());
        data.push(Value::String(event.to_string()));
        data.extend(args);
        let payload = Value::Array(data).to_string();
        let msg = encode_with_namespace(&ns, "42", &payload);
        self.send_outgoing(OutgoingPacket::Text(msg))
    }

    /// 往默认命名空间发送 JSON 数据事件，并等待服务端发回 Ack 响应或超时。
    /// Emits a JSON data event to the default namespace, and waits for an Ack response from the server or times out.
    pub async fn emit_with_ack<T: Serialize>(
        &self,
        event: &str,
        data: T,
        timeout: Duration,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        self.emit_with_ack_ns(&self.namespace, event, data, timeout)
            .await
    }

    /// 往指定命名空间发送 JSON 数据事件，并等待服务端发回 Ack 响应或超时。
    /// Emits a JSON data event to the specified namespace, and waits for an Ack response from the server or times out.
    pub async fn emit_with_ack_ns<T: Serialize>(
        &self,
        ns: &str,
        event: &str,
        data: T,
        timeout: Duration,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let ns = normalize_ns(ns);
        let ack_id = self.ack_id_counter.fetch_add(1, Ordering::Relaxed);
        let payload = json!([event, data]).to_string();
        let msg = encode_with_namespace_with_id(&ns, "42", ack_id, &payload);

        let (tx, rx) = oneshot::channel();
        {
            let mut guard = self.ns_states.write().unwrap();
            let st = guard.entry(ns.clone()).or_default();
            st.ack_waiters.insert(ack_id, tx);
        }

        let cleanup = || {
            let mut guard = self.ns_states.write().unwrap();
            if let Some(st) = guard.get_mut(&ns) {
                st.ack_waiters.remove(&ack_id);
            }
        };

        if let Err(e) = self.send_outgoing(OutgoingPacket::Text(msg)) {
            cleanup();
            return Err(e);
        }

        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(val)) => Ok(val),
            _ => {
                cleanup();
                Err(Box::new(std::io::Error::new(std::io::ErrorKind::TimedOut, "ack timeout")))
            }
        }
    }

    /// 发送 Ack 响应报文。
    /// Sends an Ack response packet.
    pub async fn send_ack(
        &self,
        ns: &str,
        id: u64,
        data: Value,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let ns = normalize_ns(ns);
        let arr = if data.is_array() {
            data
        } else if data.is_null() {
            Value::Array(vec![])
        } else {
            Value::Array(vec![data])
        };
        let msg = encode_with_namespace_with_id(&ns, "43", id, &arr.to_string());
        self.send_outgoing(OutgoingPacket::Text(msg))
    }

    /// 往默认命名空间发送带二进制附件的事件。
    /// Emits an event with binary attachments to the default namespace.
    pub async fn emit_binary(
        &self,
        event: &str,
        args: Vec<Value>,
        attachments: Vec<Vec<u8>>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.emit_binary_ns(&self.namespace, event, args, attachments).await
    }

    /// 往指定命名空间发送带二进制附件的事件。
    /// Emits an event with binary attachments to the specified namespace.
    pub async fn emit_binary_ns(
        &self,
        ns: &str,
        event: &str,
        args: Vec<Value>,
        attachments: Vec<Vec<u8>>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let ns = normalize_ns(ns);
        let n = attachments.len();

        let mut data: Vec<Value> = Vec::with_capacity(1 + args.len() + n);
        data.push(Value::String(event.to_string()));
        data.extend(args);
        for i in 0..n {
            data.push(json!({ "_placeholder": true, "num": i }));
        }
        let payload = Value::Array(data).to_string();
        let header = encode_binary_header(&ns, "45", n, None, &payload);

        self.send_outgoing(OutgoingPacket::Text(header))?;
        for bin in attachments {
            self.send_outgoing(OutgoingPacket::Binary(bin))?;
        }
        Ok(())
    }

    /// 往默认命名空间发送带二进制附件的事件，并等待服务端发回 Ack 响应或超时。
    pub async fn emit_binary_with_ack(
        &self,
        event: &str,
        args: Vec<Value>,
        attachments: Vec<Vec<u8>>,
        timeout: Duration,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        self.emit_binary_with_ack_ns(&self.namespace, event, args, attachments, timeout).await
    }

    /// 往指定命名空间发送带二进制附件的事件，并等待服务端发回 Ack 响应或超时。
    pub async fn emit_binary_with_ack_ns(
        &self,
        ns: &str,
        event: &str,
        args: Vec<Value>,
        attachments: Vec<Vec<u8>>,
        timeout: Duration,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let ns = normalize_ns(ns);
        let ack_id = self.ack_id_counter.fetch_add(1, Ordering::Relaxed);
        let n = attachments.len();

        let mut data: Vec<Value> = Vec::with_capacity(1 + args.len() + n);
        data.push(Value::String(event.to_string()));
        data.extend(args);
        for i in 0..n {
            data.push(json!({ "_placeholder": true, "num": i }));
        }
        let payload = Value::Array(data).to_string();
        let header = encode_binary_header(&ns, "45", n, Some(ack_id), &payload);

        let (tx, rx) = oneshot::channel();
        {
            let mut guard = self.ns_states.write().unwrap();
            let st = guard.entry(ns.clone()).or_default();
            st.ack_waiters.insert(ack_id, tx);
        }

        let cleanup = || {
            let mut guard = self.ns_states.write().unwrap();
            if let Some(st) = guard.get_mut(&ns) {
                st.ack_waiters.remove(&ack_id);
            }
        };

        if let Err(e) = self.send_outgoing(OutgoingPacket::Text(header)) {
            cleanup();
            return Err(e);
        }
        for bin in attachments {
            if let Err(e) = self.send_outgoing(OutgoingPacket::Binary(bin)) {
                cleanup();
                return Err(e);
            }
        }

        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(val)) => Ok(val),
            _ => {
                cleanup();
                Err(Box::new(std::io::Error::new(std::io::ErrorKind::TimedOut, "ack timeout")))
            }
        }
    }

    /// 主动断开指定命名空间的连接。
    /// Actively disconnects the specified namespace.
    pub async fn disconnect_ns(&self, ns: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let ns = normalize_ns(ns);
        let msg = if ns == "/" {
            "41".to_string()
        } else {
            format!("41{ns},")
        };
        self.send_outgoing(OutgoingPacket::Text(msg))?;
        {
            let mut guard = self.ns_states.write().unwrap();
            if let Some(st) = guard.get_mut(&ns) {
                st.connected = false;
                st.sid = None;
            }
        }
        
        let evt = IncomingEvent {
            data: Value::Null,
            ack_id: None,
            namespace: ns.clone(),
        };
        dispatch_to_ns(&self.ns_states, &ns, SYS_DISCONNECT, evt).await;
        
        Ok(())
    }

    /// 关闭底层传输连接（即断开整个 Socket.IO 客户端）。
    /// Closes the underlying transport connection (i.e., disconnects the entire Socket.IO client).
    pub async fn close(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let _ = self.send_outgoing(OutgoingPacket::Text("1".to_string()));
        // 关闭 outgoing 通道，writer 收到 None 后自行退出
        {
            let mut guard = self.outgoing_tx.write().unwrap();
            *guard = None;
        }
        if let Some(h) = self.writer_task.lock().await.take() {
            h.abort();
        }
        if let Some(h) = self.reader_task.lock().await.take() {
            h.abort();
        }
        Ok(())
    }

    /// 在连接建立之后，动态加入并连接一个新的命名空间。
    /// Dynamically joins and connects to a new namespace after the connection is established.
    pub async fn connect_additional_ns(
        &self,
        ns: &str,
        auth: Option<Value>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let ns = normalize_ns(ns);
        self.ensure_ns(&ns);
        if let Some(auth) = auth.clone() {
            let mut guard = self.ns_states.write().unwrap();
            if let Some(st) = guard.get_mut(&ns) {
                st.auth = Some(auth);
            }
        }
        let msg = format_connect_packet(&ns, auth);
        self.send_outgoing(OutgoingPacket::Text(msg))
    }

    /// 向默认命名空间发送加入房间的请求。需要服务端有相应的业务逻辑配合。
    /// Sends a request to join a room in the default namespace. Requires corresponding business logic on the server.
    pub async fn join(&self, room: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.emit("join", json!({"room": room})).await
    }

    /// 向默认命名空间发送离开房间的请求。需要服务端有相应的业务逻辑配合。
    /// Sends a request to leave a room in the default namespace. Requires corresponding business logic on the server.
    pub async fn leave(&self, room: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.emit("leave", json!({"room": room})).await
    }

    // -------------------------
    // 工具
    // -------------------------
    fn send_outgoing(&self, pkt: OutgoingPacket) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let tx = {
            let guard = self.outgoing_tx.read().unwrap();
            guard.clone()
        };
        let tx = tx.ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotConnected, "not connected")
        })?;
        tx.send(pkt)
            .map_err(|_| -> Box<dyn std::error::Error + Send + Sync> { "transport closed".into() })?;
        Ok(())
    }
}

// ---------------------------------------------------------
// 事件处理循环（与传输无关，读 IncomingFrame）
// ---------------------------------------------------------

async fn run_event_processor(
    mut incoming_rx: mpsc::UnboundedReceiver<IncomingFrame>,
    ns_states: StdArc<RwLock<HashMap<String, NsState>>>,
    outgoing_tx: mpsc::UnboundedSender<OutgoingPacket>,
    last_ping: StdArc<Mutex<Instant>>,
) {
    let mut pending_binary: Option<PendingBinary> = None;
    let mut disconnected_called = false;

    while let Some(frame) = incoming_rx.recv().await {
        match frame {
            IncomingFrame::Text(s) => {
                if s.is_empty() {
                    continue;
                }
                let bytes = s.as_bytes();
                let eio_type = bytes[0];
                match eio_type {
                    // ping（2 或 2probe）
                    b'2' => {
                        *last_ping.lock().await = Instant::now();
                        let reply = if s == "2" {
                            "3".to_string()
                        } else {
                            format!("3{}", &s[1..])
                        };
                        let _ = outgoing_tx.send(OutgoingPacket::Text(reply));
                        continue;
                    }
                    b'3' => continue, // 服务端通常不发 pong
                    b'1' => {
                        broadcast_system_event(&ns_states, SYS_DISCONNECT, Value::Null).await;
                        disconnected_called = true;
                        break;
                    }
                    b'0' | b'5' | b'6' => continue,
                    b'4' => {
                        let sio = &s[1..];
                        if sio.is_empty() {
                            continue;
                        }
                        let sio_type = sio.as_bytes()[0];
                        let rest = &sio[1..];
                        handle_socketio_packet(
                            &ns_states,
                            &outgoing_tx,
                            sio_type,
                            rest,
                            &mut pending_binary,
                        )
                        .await;
                    }
                    _ => {}
                }
            }
            IncomingFrame::Binary(bin) => {
                if let Some(mut pb) = pending_binary.take() {
                    pb.attachments.push(bin);
                    if pb.attachments.len() >= pb.num_expected {
                        finalize_binary(&ns_states, pb).await;
                    } else {
                        pending_binary = Some(pb);
                    }
                }
            }
        }
    }

    if !disconnected_called {
        broadcast_system_event(&ns_states, SYS_DISCONNECT, Value::Null).await;
    }
}

async fn handle_socketio_packet(
    ns_states: &StdArc<RwLock<HashMap<String, NsState>>>,
    _outgoing_tx: &mpsc::UnboundedSender<OutgoingPacket>,
    sio_type: u8,
    rest: &str,
    pending_binary: &mut Option<PendingBinary>,
) {
    match sio_type {
        b'0' => {
            let (packet_ns, payload_str) = parse_ns_payload(rest);
            let sid = serde_json::from_str::<Value>(payload_str)
                .ok()
                .and_then(|v| v.get("sid").and_then(|s| s.as_str()).map(|s| s.to_string()));
            {
                let mut guard = ns_states.write().unwrap();
                let st = guard.entry(packet_ns.clone()).or_default();
                st.sid = sid;
                st.connected = true;
            }
            dispatch_to_ns(
                ns_states,
                &packet_ns,
                SYS_CONNECT,
                IncomingEvent {
                    data: Value::Null,
                    ack_id: None,
                    namespace: packet_ns.clone(),
                },
            )
            .await;
        }
        b'1' => {
            let (packet_ns, _) = parse_ns_payload(rest);
            {
                let mut guard = ns_states.write().unwrap();
                if let Some(st) = guard.get_mut(&packet_ns) {
                    st.connected = false;
                    st.sid = None;
                }
            }
            dispatch_to_ns(
                ns_states,
                &packet_ns,
                SYS_DISCONNECT,
                IncomingEvent {
                    data: Value::Null,
                    ack_id: None,
                    namespace: packet_ns.clone(),
                },
            )
            .await;
        }
        b'2' => {
            let (packet_ns, ack_id, payload_str) = parse_ns_payload_with_id(rest);
            if let Ok(arr) = serde_json::from_str::<Value>(payload_str)
                && let (Some(evt), Some(args)) = (
                    arr.get(0).and_then(|v| v.as_str()).map(str::to_string),
                    arr.as_array()
                        .map(|a| Value::Array(a.iter().skip(1).cloned().collect())),
                )
            {
                dispatch_to_ns(
                    ns_states,
                    &packet_ns,
                    &evt,
                    IncomingEvent {
                        data: args,
                        ack_id,
                        namespace: packet_ns.clone(),
                    },
                )
                .await;
            }
        }
        b'3' => {
            let (packet_ns, id_opt, payload_str) = parse_ns_payload_with_id(rest);
            let payload =
                serde_json::from_str::<Value>(payload_str).unwrap_or(Value::Null);
            if let Some(id) = id_opt {
                let tx = {
                    let mut guard = ns_states.write().unwrap();
                    guard
                        .get_mut(&packet_ns)
                        .and_then(|st| st.ack_waiters.remove(&id))
                };
                if let Some(tx) = tx {
                    let _ = tx.send(payload);
                }
            }
        }
        b'4' => {
            let (packet_ns, payload_str) = parse_ns_payload(rest);
            let err_val = serde_json::from_str::<Value>(payload_str)
                .unwrap_or(Value::String(payload_str.to_string()));
            {
                let mut guard = ns_states.write().unwrap();
                if let Some(st) = guard.get_mut(&packet_ns) {
                    st.connected = false;
                }
            }
            dispatch_to_ns(
                ns_states,
                &packet_ns,
                SYS_CONNECT_ERROR,
                IncomingEvent {
                    data: err_val.clone(),
                    ack_id: None,
                    namespace: packet_ns.clone(),
                },
            )
            .await;
            dispatch_to_ns(
                ns_states,
                &packet_ns,
                SYS_ERROR,
                IncomingEvent {
                    data: err_val,
                    ack_id: None,
                    namespace: packet_ns.clone(),
                },
            )
            .await;
        }
        b'5' | b'6' => {
            let is_ack = sio_type == b'6';
            let (num_attach, after_n) = parse_binary_header(rest);
            let (packet_ns, id_opt, payload_str) = parse_ns_payload_with_id(after_n);
            if let Ok(tmpl) = serde_json::from_str::<Value>(payload_str) {
                let pb = PendingBinary {
                    ns: packet_ns,
                    is_ack,
                    ack_id: id_opt,
                    template: tmpl,
                    num_expected: num_attach,
                    attachments: Vec::with_capacity(num_attach),
                };
                if num_attach == 0 {
                    finalize_binary(ns_states, pb).await;
                } else {
                    *pending_binary = Some(pb);
                }
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------
// WebSocket reader / writer
// ---------------------------------------------------------

async fn ws_reader_loop(
    mut stream: futures::stream::SplitStream<WsStream>,
    incoming_tx: mpsc::UnboundedSender<IncomingFrame>,
) {
    while let Some(msg) = stream.next().await {
        match msg {
            Ok(Message::Text(t)) => {
                let s: String = t.to_string();
                if incoming_tx.send(IncomingFrame::Text(s)).is_err() {
                    break;
                }
            }
            Ok(Message::Binary(b)) => {
                let v: Vec<u8> = AsRef::<[u8]>::as_ref(&b).to_vec();
                if incoming_tx.send(IncomingFrame::Binary(v)).is_err() {
                    break;
                }
            }
            Ok(Message::Close(_)) => break,
            Ok(_) => {}
            Err(_) => break,
        }
    }
}

async fn ws_writer_loop(
    mut sink: futures::stream::SplitSink<WsStream, Message>,
    mut outgoing_rx: mpsc::UnboundedReceiver<OutgoingPacket>,
) {
    while let Some(pkt) = outgoing_rx.recv().await {
        let res = match pkt {
            OutgoingPacket::Text(s) => sink.send(Message::Text(s)).await,
            OutgoingPacket::Binary(b) => sink.send(Message::Binary(b)).await,
        };
        if res.is_err() {
            break;
        }
    }
    let _ = sink.close().await;
}

// ---------------------------------------------------------
// Polling reader / writer
// ---------------------------------------------------------

/// 长轮询 GET 循环：每次请求若有数据即返回，解包后送入 incoming 通道并立即发起下一轮。
async fn polling_reader_loop(
    http: reqwest::Client,
    base_url: String,
    sid: String,
    incoming_tx: mpsc::UnboundedSender<IncomingFrame>,
) {
    let url = format!(
        "{base_url}/socket.io/?EIO=4&transport=polling&sid={sid}"
    );
    loop {
        let resp = match http.get(&url).send().await {
            Ok(r) => r,
            Err(e) => {
                eprintln!("[socket_io_client] polling GET error: {e}");
                break;
            }
        };
        if !resp.status().is_success() {
            eprintln!(
                "[socket_io_client] polling GET status {}, stopping",
                resp.status()
            );
            break;
        }
        let body = match resp.text().await {
            Ok(b) => b,
            Err(e) => {
                eprintln!("[socket_io_client] polling GET body error: {e}");
                break;
            }
        };

        for part in body.split('\u{1e}') {
            if part.is_empty() {
                continue;
            }
            if let Some(rest) = part.strip_prefix('b') {
                // 二进制附件（base64）
                match general_purpose::STANDARD.decode(rest) {
                    Ok(bin) => {
                        if incoming_tx.send(IncomingFrame::Binary(bin)).is_err() {
                            return;
                        }
                    }
                    Err(e) => {
                        eprintln!("[socket_io_client] polling base64 decode failed: {e}");
                    }
                }
            } else if incoming_tx
                .send(IncomingFrame::Text(part.to_string()))
                .is_err()
            {
                return;
            }
        }
    }
}

/// 长轮询 POST 循环：批量发送当前通道里所有待发包。
async fn polling_writer_loop(
    http: reqwest::Client,
    base_url: String,
    sid: String,
    mut outgoing_rx: mpsc::UnboundedReceiver<OutgoingPacket>,
) {
    let url = format!(
        "{base_url}/socket.io/?EIO=4&transport=polling&sid={sid}"
    );
    loop {
        let first = match outgoing_rx.recv().await {
            Some(p) => p,
            None => break,
        };

        let mut body = String::new();
        append_polling_packet(&first, &mut body);
        while let Ok(more) = outgoing_rx.try_recv() {
            body.push('\u{1e}');
            append_polling_packet(&more, &mut body);
        }

        let res = http
            .post(&url)
            .header("Content-Type", "text/plain;charset=UTF-8")
            .body(body)
            .send()
            .await;
        match res {
            Ok(r) if r.status().is_success() => { /* ok */ }
            Ok(r) => {
                eprintln!(
                    "[socket_io_client] polling POST status {}, stopping",
                    r.status()
                );
                break;
            }
            Err(e) => {
                eprintln!("[socket_io_client] polling POST error: {e}");
                break;
            }
        }
    }
}

#[doc(hidden)]
pub fn append_polling_packet(p: &OutgoingPacket, out: &mut String) {
    match p {
        OutgoingPacket::Text(s) => out.push_str(s),
        OutgoingPacket::Binary(b) => {
            out.push('b');
            out.push_str(&general_purpose::STANDARD.encode(b));
        }
    }
}

// ---------------------------------------------------------
// 通用辅助
// ---------------------------------------------------------

#[doc(hidden)]
pub fn normalize_ns(ns: &str) -> String {
    if ns.is_empty() || ns == "/" {
        "/".to_string()
    } else if ns.starts_with('/') {
        ns.to_string()
    } else {
        format!("/{ns}")
    }
}

#[doc(hidden)]
pub fn format_connect_packet(ns: &str, auth: Option<Value>) -> String {
    let payload = match auth {
        Some(v) => v.to_string(),
        None => String::new(),
    };
    if ns == "/" {
        format!("40{payload}")
    } else if payload.is_empty() {
        format!("40{ns},")
    } else {
        format!("40{ns},{payload}")
    }
}

async fn dispatch_to_ns(
    ns_states: &StdArc<RwLock<HashMap<String, NsState>>>,
    ns: &str,
    event: &str,
    payload: IncomingEvent,
) {
    let cbs_opt = {
        let guard = ns_states.read().unwrap();
        guard
            .get(ns)
            .and_then(|st| st.event_handlers.get(event).cloned())
    };
    if let Some(cbs) = cbs_opt {
        for cb in cbs {
            let fut = cb(payload.clone());
            tokio::spawn(async move {
                fut.await;
            });
        }
    }
}

async fn broadcast_system_event(
    ns_states: &StdArc<RwLock<HashMap<String, NsState>>>,
    event: &str,
    data: Value,
) {
    let snapshot: Vec<(String, Vec<AsyncEventCallback>)> = {
        let guard = ns_states.read().unwrap();
        guard
            .iter()
            .filter_map(|(ns, st)| {
                st.event_handlers
                    .get(event)
                    .cloned()
                    .map(|cbs| (ns.clone(), cbs))
            })
            .collect()
    };
    for (ns, cbs) in snapshot {
        for cb in cbs {
            let fut = cb(IncomingEvent {
                data: data.clone(),
                ack_id: None,
                namespace: ns.clone(),
            });
            tokio::spawn(async move {
                fut.await;
            });
        }
    }
}

async fn finalize_binary(
    ns_states: &StdArc<RwLock<HashMap<String, NsState>>>,
    pb: PendingBinary,
) {
    let PendingBinary {
        ns,
        is_ack,
        ack_id,
        mut template,
        attachments,
        ..
    } = pb;

    substitute_placeholders(&mut template, &attachments);

    if is_ack {
        if let Some(id) = ack_id {
            let tx = {
                let mut guard = ns_states.write().unwrap();
                guard.get_mut(&ns).and_then(|st| st.ack_waiters.remove(&id))
            };
            if let Some(tx) = tx {
                let _ = tx.send(template);
            }
        }
    } else if let Some(arr) = template.as_array()
        && let Some(evt) = arr.first().and_then(|v| v.as_str()).map(str::to_string) {
            let args = Value::Array(arr.iter().skip(1).cloned().collect());
            dispatch_to_ns(
                ns_states,
                &ns,
                &evt,
                IncomingEvent {
                    data: args,
                    ack_id,
                    namespace: ns.clone(),
                },
            )
            .await;
        }
}

fn substitute_placeholders(v: &mut Value, attachments: &[Vec<u8>]) {
    match v {
        Value::Object(map) => {
            if map.get("_placeholder").and_then(|p| p.as_bool()) == Some(true)
                && let Some(n) = map.get("num").and_then(|n| n.as_u64())
                    && let Some(bin) = attachments.get(n as usize)
                {
                    let encoded = general_purpose::STANDARD.encode(bin);
                    let mut replacement = serde_json::Map::new();
                    replacement.insert("binary".to_string(), Value::String(encoded));
                    *v = Value::Object(replacement);
                    return;
                }
            for (_, val) in map.iter_mut() {
                substitute_placeholders(val, attachments);
            }
        }
        Value::Array(arr) => {
            for item in arr.iter_mut() {
                substitute_placeholders(item, attachments);
            }
        }
        _ => {}
    }
}

