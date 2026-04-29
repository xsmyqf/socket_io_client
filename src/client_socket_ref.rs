//! 客户端 Socket 引用组件。
//! Client Socket reference component.
//!
//! 提供类似服务端 `SocketRef` 的封装，将底层 `SocketIOClient` 和特定的 `Namespace` 绑定，
//! 使得在处理特定 Namespace 的事件时，能够直接进行事件的触发和响应。
//! Provides an encapsulation similar to the server-side `SocketRef`, binding the underlying `SocketIOClient` with a specific `Namespace`,
//! making it possible to directly trigger and respond to events when handling events in a specific Namespace.

use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc as StdArc, Mutex as StdMutex};
use std::time::Duration;
use tokio::task::JoinHandle;

use super::{
    extractor::Extractor,
    socket_io_client::{IncomingEvent, SocketIOClient, SYS_CONNECT, SYS_CONNECT_ERROR,
        SYS_DISCONNECT, SYS_ERROR, SYS_CLOSED, SYS_RECONNECT_ATTEMPT, SYS_RECONNECT},
};

// =====================================================
// ClientInner —— 底层状态容器
// =====================================================

/// 底层客户端状态容器。
/// Underlying client state container.
///
/// 封装了 `SocketIOClient`、应用状态以及管理各个命名空间下的定时任务。
/// Encapsulates `SocketIOClient`, application state, and manages periodic tasks under each namespace.
pub struct ClientInner<S> {
    /// 核心的 Socket.IO 客户端实例
    /// Core Socket.IO client instance
    pub client: StdMutex<SocketIOClient>,
    /// 用户自定义的全局应用状态
    /// User-defined global application state
    pub app_state: StdArc<S>,
    /// 记录各命名空间启动的周期性任务，以便在断开时中止
    /// Records periodic tasks started by each namespace to abort them upon disconnection
    pub interval_tasks: StdMutex<HashMap<String, JoinHandle<()>>>,
}

// =====================================================
// ClientSocketRef —— 客户端版 SocketRef（绑定到一个 ns）
// =====================================================

/// 绑定了特定命名空间的 Socket 引用。
/// Socket reference bound to a specific namespace.
///
/// 通过它可进行该命名空间内的事件分发、消息发送及房间操作等。
/// Through this, event dispatching, message sending, and room operations within that namespace can be performed.
pub struct ClientSocketRef<S> {
    /// 共享的底层状态容器
    /// Shared underlying state container
    pub inner: StdArc<ClientInner<S>>,
    /// 当前绑定的命名空间（例如 `/` 或 `/admin`）
    /// Currently bound namespace (e.g., `/` or `/admin`)
    pub namespace: String,
}

impl<S> Clone for ClientSocketRef<S> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            namespace: self.namespace.clone(),
        }
    }
}

impl<S> ClientSocketRef<S>
where
    S: Send + Sync + 'static,
{
    /// 使用底层容器和默认 Namespace 创建 `ClientSocketRef`
    /// Creates a `ClientSocketRef` using the underlying container and default Namespace
    pub fn new(inner: StdArc<ClientInner<S>>) -> Self {
        let ns = {
            let c = inner.client.lock().unwrap();
            c.namespace.clone()
        };
        Self {
            inner,
            namespace: ns,
        }
    }

    /// 使用底层容器和指定的 Namespace 创建 `ClientSocketRef`
    /// Creates a `ClientSocketRef` using the underlying container and specified Namespace
    pub fn with_namespace(inner: StdArc<ClientInner<S>>, ns: String) -> Self {
        Self { inner, namespace: ns }
    }

    /// 获取 Socket 唯一标识（目前客户端固定为 "client"）
    /// Gets the unique Socket identifier (currently fixed to "client" for the client)
    pub fn id(&self) -> String {
        "client".into()
    }

    /// 获取当前绑定的命名空间
    /// Gets the currently bound namespace
    pub fn namespace(&self) -> String {
        self.namespace.clone()
    }

    /// 获取当前底层使用的传输协议模式（Polling 或 WebSocket）
    /// Gets the transport mode currently used by the underlying connection (Polling or WebSocket)
    pub fn transport(&self) -> super::socket_io_client::TransportMode {
        self.clone_client().transport()
    }

    // =====================================================
    // emit 系列
    // =====================================================

    /// 发送 JSON 数据给服务端当前 Namespace 的对应事件。
    /// Emits JSON data to the corresponding event in the current Namespace on the server.
    pub async fn emit<T: Serialize>(
        &self,
        event: &str,
        data: T,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let client = self.clone_client();
        client.emit_ns(&self.namespace, event, data).await
    }

    /// 发送 JSON 数据给服务端，并等待服务端执行 `ack` 回调返回的结果。
    /// Emits JSON data to the server, and waits for the result returned by the server's `ack` callback.
    pub async fn emit_with_ack<T: Serialize>(
        &self,
        event: &str,
        data: T,
        timeout: Duration,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let client = self.clone_client();
        client
            .emit_with_ack_ns(&self.namespace, event, data, timeout)
            .await
    }

    /// 发送二进制数据（及关联的参数）到服务端。
    /// Emits binary data (and associated parameters) to the server.
    pub async fn emit_binary(
        &self,
        event: &str,
        args: Vec<Value>,
        attachments: Vec<Vec<u8>>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let client = self.clone_client();
        client
            .emit_binary_ns(&self.namespace, event, args, attachments)
            .await
    }

    /// 发送二进制数据（及关联的参数）到服务端，并等待 Ack。
    pub async fn emit_binary_with_ack(
        &self,
        event: &str,
        args: Vec<Value>,
        attachments: Vec<Vec<u8>>,
        timeout: Duration,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let client = self.clone_client();
        client
            .emit_binary_with_ack_ns(&self.namespace, event, args, attachments, timeout)
            .await
    }

    /// 发送多参数事件到服务端。
    /// Emits a multi-argument event to the server.
    pub async fn emit_args(
        &self,
        event: &str,
        args: Vec<Value>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let client = self.clone_client();
        client.emit_args_ns(&self.namespace, event, args).await
    }

    /// 断开当前 Namespace 的连接。
    /// Disconnects the current Namespace.
    pub async fn disconnect(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let client = self.clone_client();
        client.disconnect_ns(&self.namespace).await
    }

    /// 加入指定的房间（需要服务端处理对应逻辑）。
    /// Joins the specified room (requires corresponding logic handling on the server).
    pub async fn join(&self, room: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.emit("join", serde_json::json!({"room": room})).await
    }

    /// 离开指定的房间（需要服务端处理对应逻辑）。
    /// Leaves the specified room (requires corresponding logic handling on the server).
    pub async fn leave(&self, room: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.emit("leave", serde_json::json!({"room": room})).await
    }

    fn clone_client(&self) -> SocketIOClient {
        let guard = self.inner.client.lock().unwrap();
        guard.clone()
    }

    // =====================================================
    // 核心：socketioxide 风格 on<E1,E2,E3>
    // =====================================================

    /// 注册一个通用事件处理器。
    /// Registers a general event handler.
    ///
    /// 支持利用提取器（`Extractor`）机制自动将请求数据解析为所需的类型，
    /// Supports using the extractor (`Extractor`) mechanism to automatically parse request data into the required type,
    /// 例如 `Data<T>` 或 `State<S>`。最多支持 3 个参数。
    /// e.g., `Data<T>` or `State<S>`. Supports up to 3 parameters.
    pub fn on<E1, E2, E3, H, Fut>(&self, event: &str, handler: H)
    where
        E1: Extractor<S> + Send + 'static,
        E2: Extractor<S> + Send + 'static,
        E3: Extractor<S> + Send + 'static,
        H: Fn(E1, E2, E3) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        let handler = StdArc::new(handler);
        self.register(event, move |socket, evt| {
            let handler = handler.clone();
            Box::pin(async move {
                let e1 = E1::extract(&socket, &evt);
                let e2 = E2::extract(&socket, &evt);
                let e3 = E3::extract(&socket, &evt);
                handler(e1, e2, e3).await;
            })
        });
    }

    // =====================================================
    // 系统事件：on_connect / on_disconnect / on_error / on_connect_error / on_closed
    // =====================================================

    /// 注册 `connect` 系统事件处理器。
    /// Registers the `connect` system event handler.
    pub fn on_connect<E1, E2, H, Fut>(&self, handler: H)
    where
        E1: Extractor<S> + Send + 'static,
        E2: Extractor<S> + Send + 'static,
        H: Fn(E1, E2) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        self.register_sys_2(SYS_CONNECT, handler);
    }

    /// 注册 `disconnect` 系统事件处理器。
    /// Registers the `disconnect` system event handler.
    pub fn on_disconnect<E1, E2, H, Fut>(&self, handler: H)
    where
        E1: Extractor<S> + Send + 'static,
        E2: Extractor<S> + Send + 'static,
        H: Fn(E1, E2) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        self.register_sys_2(SYS_DISCONNECT, handler);
    }

    /// 注册 `closed` 系统事件处理器（当连接被显式关闭时触发）。
    /// Registers the `closed` system event handler (triggered when the connection is explicitly closed).
    pub fn on_closed<E1, E2, H, Fut>(&self, handler: H)
    where
        E1: Extractor<S> + Send + 'static,
        E2: Extractor<S> + Send + 'static,
        H: Fn(E1, E2) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        self.register_sys_2(SYS_CLOSED, handler);
    }

    /// 注册 `error` 系统事件处理器。
    /// Registers the `error` system event handler.
    pub fn on_error<E1, E2, H, Fut>(&self, handler: H)
    where
        E1: Extractor<S> + Send + 'static,
        E2: Extractor<S> + Send + 'static,
        H: Fn(E1, E2, String) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        let handler = StdArc::new(handler);
        self.register(SYS_ERROR, move |socket, evt| {
            let handler = handler.clone();
            Box::pin(async move {
                let e1 = E1::extract(&socket, &evt);
                let e2 = E2::extract(&socket, &evt);
                let err = match &evt.data {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                handler(e1, e2, err).await;
            })
        });
    }

    /// 注册 `connect_error` 系统事件处理器（连接失败时触发）。
    /// Registers the `connect_error` system event handler (triggered when connection fails).
    pub fn on_connect_error<E1, E2, H, Fut>(&self, handler: H)
    where
        E1: Extractor<S> + Send + 'static,
        E2: Extractor<S> + Send + 'static,
        H: Fn(E1, E2, Value) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        let handler = StdArc::new(handler);
        self.register(SYS_CONNECT_ERROR, move |socket, evt| {
            let handler = handler.clone();
            Box::pin(async move {
                let e1 = E1::extract(&socket, &evt);
                let e2 = E2::extract(&socket, &evt);
                handler(e1, e2, evt.data.clone()).await;
            })
        });
    }

    /// 注册 `reconnect_attempt` 系统事件处理器（尝试重连时触发，包含当前重连次数）。
    /// Registers the `reconnect_attempt` system event handler (triggered on a reconnection attempt, contains the current retry count).
    pub fn on_reconnect_attempt<E1, E2, H, Fut>(&self, handler: H)
    where
        E1: Extractor<S> + Send + 'static,
        E2: Extractor<S> + Send + 'static,
        H: Fn(E1, E2, u64) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        let handler = StdArc::new(handler);
        self.register(SYS_RECONNECT_ATTEMPT, move |socket, evt| {
            let handler = handler.clone();
            Box::pin(async move {
                let e1 = E1::extract(&socket, &evt);
                let e2 = E2::extract(&socket, &evt);
                let attempts = evt.data.as_u64().unwrap_or(0);
                handler(e1, e2, attempts).await;
            })
        });
    }

    /// 注册 `reconnect` 系统事件处理器（重连成功时触发，包含重连尝试的次数）。
    /// Registers the `reconnect` system event handler (triggered on successful reconnection, contains the number of attempts made).
    pub fn on_reconnect<E1, E2, H, Fut>(&self, handler: H)
    where
        E1: Extractor<S> + Send + 'static,
        E2: Extractor<S> + Send + 'static,
        H: Fn(E1, E2, u64) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        let handler = StdArc::new(handler);
        self.register(SYS_RECONNECT, move |socket, evt| {
            let handler = handler.clone();
            Box::pin(async move {
                let e1 = E1::extract(&socket, &evt);
                let e2 = E2::extract(&socket, &evt);
                let attempts = evt.data.as_u64().unwrap_or(0);
                handler(e1, e2, attempts).await;
            })
        });
    }

    fn register_sys_2<E1, E2, H, Fut>(&self, sys_event: &str, handler: H)
    where
        E1: Extractor<S> + Send + 'static,
        E2: Extractor<S> + Send + 'static,
        H: Fn(E1, E2) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        let handler = StdArc::new(handler);
        self.register(sys_event, move |socket, evt| {
            let handler = handler.clone();
            Box::pin(async move {
                let e1 = E1::extract(&socket, &evt);
                let e2 = E2::extract(&socket, &evt);
                handler(e1, e2).await;
            })
        });
    }

    /// 内部：将一个（socket, event）→ BoxFuture 的处理器注册到底层 client。
    fn register<F>(&self, event: &str, wrap: F)
    where
        F: Fn(ClientSocketRef<S>, IncomingEvent) -> futures::future::BoxFuture<'static, ()>
            + Send
            + Sync
            + 'static,
    {
        let inner = self.inner.clone();
        let ns = self.namespace.clone();
        let wrap = StdArc::new(wrap);

        let mut client = inner.client.lock().unwrap();
        let inner_for_cb = inner.clone();
        let ns_for_cb = ns.clone();
        client.on_ns(&ns, event, move |evt: IncomingEvent| {
            let socket = ClientSocketRef::with_namespace(inner_for_cb.clone(), ns_for_cb.clone());
            (wrap)(socket, evt)
        });
    }

    // =====================================================
    // 周期任务：在 connect 后每隔 interval 执行一次 handler
    // =====================================================

    /// 启动一个周期性任务，当 `connect` 成功时自动开始执行，当 `disconnect` 时自动中止。
    /// Starts a periodic task that automatically begins execution on a successful `connect`, and automatically aborts on `disconnect`.
    ///
    /// * `loop_name`: 任务名称，用于唯一标识该任务以便管理
    /// * `loop_name`: Task name, used to uniquely identify the task for management
    /// * `interval`: 每次执行的时间间隔
    /// * `interval`: Time interval for each execution
    /// * `count`: 最大执行次数，若为 `0` 则表示无限循环
    /// * `count`: Maximum number of executions; if `0`, it loops infinitely
    /// * `handler`: 每次触发时调用的异步处理函数
    /// * `handler`: Asynchronous processing function called on each trigger
    pub fn run_interval<E1, E2, H, Fut>(
        &self,
        loop_name: &str,
        interval: Duration,
        count: usize,
        handler: H,
    ) where
        E1: Extractor<S> + Send + 'static,
        E2: Extractor<S> + Send + 'static,
        H: Fn(String, Duration, usize, E1, E2) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        let handler = StdArc::new(handler);
        let name = loop_name.to_string();

        // connect 时启动循环
        let h_connect = handler.clone();
        let name_connect = name.clone();
        self.register(SYS_CONNECT, move |socket, _evt| {
            let handler = h_connect.clone();
            let name = name_connect.clone();
            let interval_cloned = interval;
            let count_cloned = count;

            Box::pin(async move {
                let inner = socket.inner.clone();
                let socket_for_task = socket.clone();
                let name_for_cb = name.clone();

                let h = tokio::spawn(async move {
                    let mut executed: usize = 0;
                    loop {
                        if count_cloned != 0 && executed >= count_cloned {
                            break;
                        }
                        let e1 = E1::extract(
                            &socket_for_task,
                            &IncomingEvent {
                                data: Value::Null,
                                ack_id: None,
                                namespace: socket_for_task.namespace.clone(),
                            },
                        );
                        let e2 = E2::extract(
                            &socket_for_task,
                            &IncomingEvent {
                                data: Value::Null,
                                ack_id: None,
                                namespace: socket_for_task.namespace.clone(),
                            },
                        );
                        handler(name_for_cb.clone(), interval_cloned, count_cloned, e1, e2)
                            .await;
                        tokio::time::sleep(interval_cloned).await;
                        executed += 1;
                    }
                });
                let mut tasks = inner.interval_tasks.lock().unwrap();
                if let Some(prev) = tasks.remove(&name) {
                    prev.abort();
                }
                tasks.insert(name, h);
            })
        });

        // disconnect 时中止
        let name_disconnect = name.clone();
        self.register(SYS_DISCONNECT, move |socket, _evt| {
            let inner = socket.inner.clone();
            let name = name_disconnect.clone();
            Box::pin(async move {
                let mut tasks = inner.interval_tasks.lock().unwrap();
                if let Some(h) = tasks.remove(&name) {
                    h.abort();
                }
            })
        });
    }
}
