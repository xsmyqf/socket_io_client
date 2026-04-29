# socket_io_client

`socket_io_client` is a simple Socket.IO client library written in Rust.

**⚠️ Compatibility Note:** This client is currently mainly adapted and tested against the `socketioxide` server. It is dedicated to providing an ergonomic, end-to-end consistent development experience within the Rust ecosystem. It supports managing global state via `ClientApp`, allows for convenient registration of different namespaces, and adopts an event routing registration method based on an `Extractor` mechanism, making the event handler signatures more flexible and clear.

## ✨ Core Features

- **Global State & Namespace Management**: Easily manage the global state of the client through `ClientApp`, and support independent event registration for multiple namespaces (e.g., `/`, `/admin`).
- **Axum-style Extractors**: Event callback functions support flexible parameter extraction, such as `CSocket<State>`, `CData<Value>`, `CState<State>`, and `Ack`, eliminating the need to manually parse the context.
- **Comprehensive Event Sending & Receiving Mechanism**:
  - Supports `on_connect` and `on_disconnect` to listen to the connection lifecycle.
  - Supports `on` to listen for custom events.
  - Supports `emit` and `emit_args` to send JSON or multi-parameter events.
  - Supports `emit_with_ack` with an acknowledgment mechanism.
- **Large File/Binary Data Chunk Upload (Anti-Disconnect Backpressure)**: Supports `emit_binary_with_ack`, combined with the Ack mechanism to achieve natural backpressure. This prevents blocking Ping/Pong heartbeats and causing disconnections when sending large binary files (like Zip archives).
- **Scheduled Task Integration**: Provides `socket.run_interval` to easily register scheduled loop tasks tied to the Socket lifecycle.
- **Auto Reconnection**: Customizable reconnection interval (`set_reconnect_interval`) and timeout (`set_pong_timeout`).

## 🚀 Quick Start

### 1. Add Dependencies

Add the following to your `Cargo.toml`:

```toml
[dependencies]
socket_io_client = "0.1.0"
serde_json = "1.0"
tokio = { version = "1.0", features = ["full"] }
tracing = "0.1"
```

### 2. Write Client Code

```rust
use serde_json::{json, Value};
use socket_io_client::{ClientApp, ClientSocketRef, Data as CData, Socket as CSocket, State as CState};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::info;

// 1. Define your global state
#[derive(Clone)]
pub struct AppState {
    pub message_count: Arc<Mutex<usize>>,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let state = AppState {
        message_count: Arc::new(Mutex::new(0)),
    };

    // 2. Initialize ClientApp
    let mut app = ClientApp::new(state);
    app.set_reconnect_interval(3);
    app.set_pong_timeout(8);

    // 3. Register namespace and event routes
    app.ns("/", |socket: ClientSocketRef<AppState>| async move {
        
        // Listen for the connection success event
        socket.on_connect::<CSocket<AppState>, CState<AppState>, _, _>(
            |CSocket(s), CState(_)| async move {
                info!("Connected successfully!");
                let _ = s.emit("hello_server", json!({"msg": "hello from client"})).await;
            }
        );

        // Listen for the disconnection event
        socket.on_disconnect::<CSocket<AppState>, CState<AppState>, _, _>(
            |CSocket(_), CState(_)| async move {
                info!("Disconnected!");
            }
        );

        // Listen for custom events and use Extractors to extract data and state
        socket.on::<CSocket<AppState>, CData<Value>, CState<AppState>, _, _>(
            "server_msg",
            |CSocket(s), CData(data), CState(state)| async move {
                info!("Received server message: {:?}", data);
                
                let mut count = state.message_count.lock().await;
                *count += 1;
                info!("Total messages received: {}", *count);
                
                // Respond to the message
                let _ = s.emit("client_reply", json!({"reply": "got it"})).await;
            }
        );
    });

    // 4. Start and connect to the server
    app.run("http://127.0.0.1:3000").await;
}
```

## 💡 Advanced Features Examples

### Send a Request with Ack

```rust
use std::time::Duration;

// ... inside the namespace registration callback
socket.on::<CSocket<AppState>, CData<Value>, CState<AppState>, _, _>(
    "get_user_info",
    |CSocket(s), CData(data), CState(_)| async move {
        // Send a request with Ack, set timeout to 3 seconds
        match s.emit_with_ack("get_user", json!(123), Duration::from_secs(3)).await {
            Ok(res) => info!("Successfully received ACK response from server: {}", res),
            Err(e) => info!("Timeout or failed waiting for server ACK: {:?}", e),
        }
    }
);
```

### Binary Large File Chunk Upload

To avoid sending massive binary data blocking the heartbeat and causing a disconnection, it's recommended to use chunking and the Ack mechanism for natural backpressure:

```rust
// ... inside the namespace registration callback
let chunk = vec![0u8; 256 * 1024]; // 256KB chunk
let chunk_meta = json!({ "file_name": "test.zip", "chunk_index": 0 });

if let Err(e) = s.emit_binary_with_ack(
    "upload_zip_chunk",
    vec![chunk_meta],
    vec![chunk], // Binary payload
    Duration::from_secs(5),
).await {
    tracing::warn!("Chunk upload failed: {}", e);
}
```
*Tip: On the server side (e.g., socketioxide 0.18.x), you can use the `Data::<(Value, bytes::Bytes)>` extractor to simultaneously receive JSON metadata and binary chunks.*

## 🧪 Run Example Projects (Examples)

This project provides complete client and server integration testing examples in the `examples` directory.

- **`examples/server_test`**: A test server built with Leptos and `socketioxide`.
- **`examples/client_test`**: A test client built with this library.

**Steps to start the integration test:**

1. Enter the server directory and start the server:
   ```bash
   cd examples/server_test
   cargo leptos watch
   ```
2. Wait for the server to start (about 3 seconds).
3. Open a new terminal, enter the client directory and start the client:
   ```bash
   cd examples/client_test
   cargo run
   ```

## 🔗 Reference Projects

The implementation and API design of this project are mainly inspired by the official Socket.IO client and excellent libraries in the Rust ecosystem:

- [socket.io-client (Node.js)](https://github.com/socketio/socket.io/tree/main/packages/socket.io-client)
- [socketioxide (Rust Server)](https://github.com/Totodore/socketioxide)

## 📄 License

This project is open-sourced under the MIT License. See the `LICENSE` file for details.
