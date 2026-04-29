//! 参数提取器（Extractor）机制组件。
//! Parameter extractor mechanism component.
//!
//! 提供类似 `axum` 或 `socketioxide` 的参数提取机制，允许用户在事件处理函数中
//! Provides a parameter extraction mechanism similar to `axum` or `socketioxide`, allowing users to declare
//! 声明需要的参数类型（如 `Socket<S>`, `Data<T>`, `State<S>` 等），框架会自动从
//! required parameter types (such as `Socket<S>`, `Data<T>`, `State<S>`, etc.) in event handler functions.
//! 底层的 `(socket, IncomingEvent)` 上下文中提取出对应的数据并注入。
//! The framework will automatically extract the corresponding data from the underlying `(socket, IncomingEvent)` context and inject it.

use serde::de::DeserializeOwned;
use serde_json::Value;
use std::sync::Arc as StdArc;

use super::client_socket_ref::ClientSocketRef;
use super::socket_io_client::IncomingEvent;

// =====================================================
// Extractor Trait
// =====================================================

/// 核心 `Extractor` Trait。
/// Core `Extractor` Trait.
///
/// 用于将事件处理函数的参数从 `(socket, IncomingEvent)` 中提取出来。
/// Used to extract the parameters of the event handler function from `(socket, IncomingEvent)`.
/// 任何实现了此 trait 的类型都可以作为 `on` 等事件注册方法的参数。
/// Any type that implements this trait can be used as a parameter for event registration methods like `on`.
pub trait Extractor<S>: Sized + Send + 'static {
    /// 执行提取逻辑，将底层上下文转换为所需的目标类型。
    /// Executes the extraction logic, converting the underlying context into the desired target type.
    fn extract(socket: &ClientSocketRef<S>, event: &IncomingEvent) -> Self;
}

// =====================================================
// Socket<S>
// =====================================================

/// 提取当前触发事件的 `ClientSocketRef`。
/// Extracts the `ClientSocketRef` of the currently triggered event.
///
/// 通过此提取器，处理器中可以获得触发该事件的 Socket 引用，从而进行回复消息、加入房间等操作。
/// Through this extractor, the handler can obtain a reference to the Socket that triggered the event, allowing operations like replying to messages and joining rooms.
pub struct Socket<S>(pub ClientSocketRef<S>);

impl<S> Extractor<S> for Socket<S>
where
    S: Send + Sync + 'static,
{
    fn extract(socket: &ClientSocketRef<S>, _event: &IncomingEvent) -> Self {
        Socket(socket.clone())
    }
}

// =====================================================
// Data<T> —— 自动 JSON 反序列化
// =====================================================

/// 将事件载荷自动反序列化为类型 `T`。
/// Automatically deserializes the event payload into type `T`.
///
/// 对于 Socket.IO 协议解析后的事件：`event.data` 是 `[arg1, arg2, ...]` 的 JSON 数组形式。
/// For events parsed from the Socket.IO protocol: `event.data` is in the form of a JSON array `[arg1, arg2, ...]`.
///
/// 解析规则：
/// Parsing rules:
/// 1. 若 `T` 本身能反序列化整个数组（例如 `Vec<Value>`、`(A, B)`），直接按数组反序列化。
/// 1. If `T` itself can deserialize the entire array (e.g., `Vec<Value>`, `(A, B)`), deserialize it directly as an array.
/// 2. 若失败，尝试用单参数形式，即将 `data[0]` 反序列化为 `T`。
/// 2. If it fails, try a single-parameter format, i.e., deserialize `data[0]` into `T`.
/// 3. 若均失败，将 panic（响亮失败优于静默吞错）。
/// 3. If both fail, panic (loud failure is preferred over silent error swallowing).
pub struct Data<T>(pub T);

impl<S, T> Extractor<S> for Data<T>
where
    S: Send + Sync + 'static,
    T: DeserializeOwned + Send + 'static,
{
    fn extract(_socket: &ClientSocketRef<S>, event: &IncomingEvent) -> Self {
        let value = &event.data;

        // 1) 直接尝试把整个 data（可能是数组）反序列化为 T
        if let Ok(v) = serde_json::from_value::<T>(value.clone()) {
            return Data(v);
        }

        // 2) 若是数组，尝试用第一个元素反序列化
        if let Some(first) = value.as_array().and_then(|a| a.first())
            && let Ok(v) = serde_json::from_value::<T>(first.clone())
        {
            return Data(v);
        }

        panic!(
            "[socket_io_client] Data<{}> deserialize failed. raw={value}",
            std::any::type_name::<T>()
        );
    }
}

// =====================================================
// State<S>
// =====================================================

/// 提取全局状态 `S` 的引用。
/// Extracts a reference to the global state `S`.
///
/// 通过 `ClientApp::new(state)` 传入的全局状态可以通过此提取器在任意处理器中获得 `Arc<S>`。
/// The global state passed in via `ClientApp::new(state)` can be obtained as `Arc<S>` in any handler through this extractor.
pub struct State<S>(pub StdArc<S>);

impl<S> Extractor<S> for State<S>
where
    S: Send + Sync + 'static,
{
    fn extract(socket: &ClientSocketRef<S>, _event: &IncomingEvent) -> Self {
        State(socket.inner.app_state.clone())
    }
}

// =====================================================
// Ack —— 对服务端主动发来的带 ack_id 的事件做回复
// =====================================================

/// 获取对带 `ack_id` 事件的响应能力（Ack 发送器）。
/// Obtains the ability to respond to events with an `ack_id` (Ack sender).
///
/// 通过调用 `.send(Value)` 能够向服务端回发 ACK（即回复客户端已处理完成该请求的数据）。
/// Calling `.send(Value)` can send an ACK back to the server (i.e., reply with data that the client has finished processing the request).
/// 若服务端没有带 `ack_id` 发起请求，则调用 `send()` 什么都不会发生。
/// If the server initiated the request without an `ack_id`, calling `send()` will do nothing.
pub struct Ack<S> {
    socket: ClientSocketRef<S>,
    namespace: String,
    id: Option<u64>,
}

impl<S> Ack<S>
where
    S: Send + Sync + 'static,
{
    /// 获取当前事件绑定的 `ack_id`。
    /// Gets the `ack_id` bound to the current event.
    pub fn id(&self) -> Option<u64> {
        self.id
    }

    /// 发送 Ack 数据回服务端。
    /// Sends Ack data back to the server.
    pub async fn send(&self, value: Value) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if let Some(id) = self.id {
            let client = {
                let guard = self.socket.inner.client.lock().unwrap();
                guard.clone()
            };
            client.send_ack(&self.namespace, id, value).await?;
        }
        Ok(())
    }
}

impl<S> Extractor<S> for Ack<S>
where
    S: Send + Sync + 'static,
{
    fn extract(socket: &ClientSocketRef<S>, event: &IncomingEvent) -> Self {
        Ack {
            socket: socket.clone(),
            namespace: event.namespace.clone(),
            id: event.ack_id,
        }
    }
}

// =====================================================
// Raw —— 原始 IncomingEvent（方便进阶使用）
// =====================================================

/// 提取原始的 `IncomingEvent` 数据对象。
/// Extracts the raw `IncomingEvent` data object.
///
/// 当 `Data<T>` 等提取器无法满足需求时，可以直接提取原始事件，
/// When extractors like `Data<T>` cannot meet the requirements, the raw event can be extracted directly,
/// 包含解析出的 `data`, `namespace` 和 `ack_id`。
/// containing the parsed `data`, `namespace`, and `ack_id`.
pub struct Raw(pub IncomingEvent);

impl<S> Extractor<S> for Raw
where
    S: Send + Sync + 'static,
{
    fn extract(_socket: &ClientSocketRef<S>, event: &IncomingEvent) -> Self {
        Raw(event.clone())
    }
}
