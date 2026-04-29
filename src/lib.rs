//! 一个简易的 Socket.IO 客户端库。
//! A simple Socket.IO client library.
//!
//! 提供了类似 `socketioxide` 的服务端开发体验，包括通过 `ClientApp` 管理全局状态、
//! Provides a server-side development experience similar to `socketioxide`, including managing global state through `ClientApp`,
//! 注册不同命名空间（Namespace）以及基于 Extractor 机制的事件路由注册方法。
//! registering different namespaces (Namespace), and event routing registration methods based on the Extractor mechanism.
//!
//! 核心组件：
//! Core components:
//! - [`ClientApp`] - 客户端主容器，负责管理各个命名空间及断线重连逻辑。
//! - [`ClientApp`] - Main client container, responsible for managing namespaces and auto-reconnection logic.
//! - [`ClientSocketRef`] - 代表一个关联到特定命名空间的 Socket 引用，可用于发送事件及加入房间。
//! - [`ClientSocketRef`] - Represents a Socket reference associated with a specific namespace, which can be used to emit events and join rooms.
//! - `Extractor` 机制：通过 `Socket`, `Data`, `State`, `Ack` 自动提取事件参数。
//! - `Extractor` mechanism: Automatically extracts event parameters via `Socket`, `Data`, `State`, `Ack`.

/// 客户端主容器，支持全局状态及命名空间管理。
/// Main client container, supports global state and namespace management.
pub mod client_app;
/// 绑定了命名空间的 Socket 引用对象。
/// Socket reference object bound to a namespace.
pub mod client_socket_ref;
/// Socket.IO 协议编码解码函数辅助库。
/// Socket.IO protocol encoding and decoding function helper library.
pub mod codec;
/// 事件处理参数自动提取器（Extractor）。
/// Automatic event handler parameter extractors.
pub mod extractor;
/// Socket.IO 底层协议的 WebSocket 和 Polling 传输实现。
/// WebSocket and Polling transport implementation of the underlying Socket.IO protocol.
#[allow(clippy::module_inception)]
pub mod socket_io_client;

// re-exports for easy use
pub use client_app::ClientApp;
pub use client_socket_ref::ClientSocketRef;
pub use extractor::{Ack, Data, Extractor, Raw, Socket, State};
pub use socket_io_client::{IncomingEvent, SocketIOClient};

#[doc(hidden)]
pub use socket_io_client::{
    OutgoingPacket, append_polling_packet, format_connect_packet, normalize_ns,
};
