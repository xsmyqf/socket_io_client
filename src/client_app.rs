//! 客户端 App 容器组件。
//! Client App container component.
//!
//! 提供类似服务端的 `App` 结构，用于集中管理全局状态（State）、多个命名空间（Namespace）、
//! 以及统一配置 Socket.IO 客户端的连接参数和生命周期。
//! Provides an `App` structure similar to the server-side, used to centrally manage global state (State), multiple namespaces (Namespace),
//! and uniformly configure the connection parameters and lifecycle of the Socket.IO client.

use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc as StdArc, Mutex as StdMutex};

use super::{
    client_socket_ref::{ClientInner, ClientSocketRef},
    socket_io_client::SocketIOClient,
};

use futures::future::BoxFuture;

/// Namespace 初始化回调处理器的类型别名
/// Type alias for Namespace initialization callback handler
pub type NsHandler<S> = Box<dyn Fn(ClientSocketRef<S>) -> BoxFuture<'static, ()> + Send + Sync>;

/// App 管理器 —— 客户端版 namespace 容器，支持同时注册多个 namespace。
/// App manager — client-side namespace container, supporting simultaneous registration of multiple namespaces.
///
/// 用法：
///
/// ```rust,no_run
/// use socket_io_client::{ClientApp, Data, Socket, State};
/// use serde::Deserialize;
///
/// #[derive(Default)]
/// struct MyState;
///
/// #[derive(Default, Deserialize, Debug)]
/// struct Msg;
///
/// let mut app = ClientApp::new(MyState);
///
/// app.ns("/", |socket| async move {
///     socket.on::<Socket<MyState>, Data<Msg>, State<MyState>, _, _>(
///         "RegisterDevice",
///         |_socket, Data(_data), State(_st)| async move {},
///     );
/// });
///
/// app.ns("/admin", |socket| async move {
///     socket.on::<Socket<MyState>, Data<Msg>, State<MyState>, _, _>(
///         "Broadcast",
///         |_socket, Data(_data), State(_st)| async move {},
///     );
/// });
///
/// futures::executor::block_on(app.run("http://127.0.0.1:5000"));
/// ```
pub struct ClientApp<S> {
    /// 全局应用状态，所有 Namespace 和事件处理器共享
    /// Global application state, shared by all Namespaces and event handlers
    pub app_state: StdArc<S>,
    /// 注册的 Namespace 以及对应的初始化回调处理器
    /// Registered Namespaces and their corresponding initialization callback handlers
    pub ns_handlers: Vec<(String, NsHandler<S>)>,
    /// 每个 Namespace 对应的认证信息（Auth Payload）
    /// Authentication information (Auth Payload) corresponding to each Namespace
    pub auth: HashMap<String, Value>,
    /// 断线重连的时间间隔（秒）
    /// Time interval for auto-reconnection (seconds)
    pub reconnect_interval_secs: Option<u64>,
    /// 尝试重连的最大次数
    /// Maximum number of reconnection attempts
    pub retry_count: Option<usize>,
    /// 期待服务端返回 Pong 的超时时间（秒）
    /// Timeout for expecting a Pong response from the server (seconds)
    pub pong_timeout_secs: Option<u64>,
    /// 是否允许 WebSocket 升级（从长轮询升级）
    /// Whether to allow WebSocket upgrade (from long polling)
    pub allow_upgrade: Option<bool>,
}

impl<S> ClientApp<S>
where
    S: Send + Sync + 'static,
{
    /// 创建一个新的 ClientApp 实例，并初始化全局状态。
    /// Creates a new ClientApp instance and initializes the global state.
    pub fn new(state: S) -> Self {
        Self {
            app_state: StdArc::new(state),
            ns_handlers: Vec::new(),
            auth: HashMap::new(),
            reconnect_interval_secs: None,
            retry_count: None,
            pong_timeout_secs: None,
            allow_upgrade: None,
        }
    }

    /// 定义 namespace；可重复调用以注册多个 namespace，它们将通过同一条 WebSocket 复用。
    /// Defines a namespace; can be called repeatedly to register multiple namespaces, which will be multiplexed over the same WebSocket.
    pub fn ns<F, Fut>(&mut self, ns: &str, f: F)
    where
        F: Fn(ClientSocketRef<S>) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        let normalized_ns = normalize_ns(ns);
        self.ns_handlers
            .push((normalized_ns, Box::new(move |socket| Box::pin(f(socket)))));
    }

    /// 为指定 namespace 预设 CONNECT 的 auth 负载（对应 Socket.IO v5 鉴权数据）。
    /// Presets the CONNECT auth payload for the specified namespace (corresponding to Socket.IO v5 authentication data).
    pub fn set_auth(&mut self, ns: &str, auth: Value) {
        let normalized_ns = normalize_ns(ns);
        self.auth.insert(normalized_ns, auth);
    }

    /// 启动客户端（带自动重连）
    /// Starts the client (with auto-reconnection)
    pub async fn run(&mut self, url: &str) {
        // 主 ns 取第一个注册的；未注册任何 ns 时使用 "/"
        let primary_ns = self
            .ns_handlers
            .first()
            .map(|(ns, _)| ns.clone())
            .unwrap_or_else(|| "/".to_string());

        let mut client = SocketIOClient::new(url);
        client.set_namespace(&primary_ns);

        if let Some(secs) = self.reconnect_interval_secs {
            client.set_reconnect_interval(secs);
        }
        if let Some(count) = self.retry_count {
            client.set_retry_count(count);
        }
        if let Some(secs) = self.pong_timeout_secs {
            client.set_pong_timeout(secs);
        }
        if let Some(allow) = self.allow_upgrade {
            client.set_allow_upgrade(allow);
        }

        // 登记所有 ns；若有 auth 则一并设置
        for (ns, _) in &self.ns_handlers {
            client.register_namespace(ns);
            if let Some(auth) = self.auth.get(ns) {
                client.set_auth(ns, auth.clone());
            }
        }

        // 构建 inner —— 所有 handler 共用一个 ClientInner
        let inner = StdArc::new(ClientInner {
            client: StdMutex::new(client),
            app_state: self.app_state.clone(),
            interval_tasks: StdMutex::new(HashMap::new()),
        });

        // 依次调用每个 ns 的 handler，把一个绑定到该 ns 的 socket_ref 交给它
        for (ns, handler) in &self.ns_handlers {
            let socket_ref = ClientSocketRef::with_namespace(inner.clone(), ns.clone());
            handler(socket_ref).await;
        }

        // handler 注册完毕后，取出最终的 client（已包含所有 event_handlers）并启动重连
        let mut client = {
            let guard = inner.client.lock().unwrap();
            guard.clone()
        };
        client.run_with_reconnect().await;
    }

    /// 设置重连间隔时间（秒）。
    /// Sets the reconnection interval in seconds.
    pub fn set_reconnect_interval(&mut self, secs: u64) {
        self.reconnect_interval_secs = Some(secs);
    }
    /// 设置最大重连尝试次数。
    /// Sets the maximum number of reconnection attempts.
    pub fn set_retry_count(&mut self, count: usize) {
        self.retry_count = Some(count);
    }
    /// 设置 Pong 超时时间（秒），若在此时间内未收到服务端 Pong 将视为断开。
    /// Sets the Pong timeout in seconds. If a Pong is not received from the server within this time, it is considered disconnected.
    pub fn set_pong_timeout(&mut self, secs: u64) {
        self.pong_timeout_secs = Some(secs);
    }
    /// 设置是否允许从 HTTP 长轮询升级为 WebSocket 协议。
    /// Sets whether to allow upgrading from HTTP long polling to the WebSocket protocol.
    pub fn set_allow_upgrade(&mut self, allow: bool) {
        self.allow_upgrade = Some(allow);
    }
}

/// 规范化 Namespace 路径，确保以 `/` 开头。
/// Normalizes the Namespace path to ensure it starts with `/`.
fn normalize_ns(ns: &str) -> String {
    if ns.is_empty() || ns == "/" {
        "/".to_string()
    } else if ns.starts_with('/') {
        ns.to_string()
    } else {
        format!("/{ns}")
    }
}
