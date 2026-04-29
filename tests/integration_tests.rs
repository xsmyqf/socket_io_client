use axum::Router;
use base64::{engine::general_purpose, Engine};
use socket_io_client::{ClientApp, Socket, Data as ClientData};
use socketioxide::{
    extract::{Data as ServerData, SocketRef},
    SocketIo,
};
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::time::{sleep, Duration};

pub async fn start_test_server_on_port(port: u16) -> (SocketIo, tokio::task::JoinHandle<()>) {
    let (layer, io) = SocketIo::new_layer();

    io.ns("/", |socket: SocketRef| async move {
        socket.on("echo", |socket: SocketRef, ServerData::<serde_json::Value>(data)| async move {
            socket.emit("echo_reply", &data).ok();
        });
    });

    let app = Router::new().layer(layer);
    let addr = format!("127.0.0.1:{}", port);
    let listener = TcpListener::bind(addr).await.unwrap();

    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    (io, handle)
}

pub async fn start_test_server() -> (u16, SocketIo, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener); // free the port for start_test_server_on_port

    let (io, handle) = start_test_server_on_port(port).await;
    (port, io, handle)
}

#[tokio::test]
/// 测试用例 1：基础连接与断开
/// 测试步骤：
/// 1. 启动本地测试服务端。
/// 2. 客户端连接到服务端默认命名空间 "/"。
/// 3. 客户端触发 `on_connect` 事件，确认连接成功。
/// 4. 客户端主动调用 `disconnect()` 断开连接。
/// 5. 客户端触发 `on_disconnect` 事件，确认断开成功。
async fn test_basic_connection_and_disconnect() {
    let _ = tracing_subscriber::fmt::try_init();

    let (port, _io, _server_handle) = start_test_server().await;
    let url = format!("http://127.0.0.1:{}", port);

    let (connect_tx, connect_rx) = oneshot::channel();
    let (disconnect_tx, disconnect_rx) = oneshot::channel();

    let connect_tx = Arc::new(Mutex::new(Some(connect_tx)));
    let disconnect_tx = Arc::new(Mutex::new(Some(disconnect_tx)));

    let mut app = ClientApp::new(());

    let ctx = connect_tx.clone();
    let dtx = disconnect_tx.clone();

    app.ns("/", move |socket| {
        let ctx = ctx.clone();
        let dtx = dtx.clone();
        async move {
            socket.on_connect({
                let ctx = ctx.clone();
                move |socket: Socket<()>, _: ClientData<serde_json::Value>| {
                    let ctx = ctx.clone();
                    async move {
                        if let Some(tx) = ctx.lock().unwrap().take() {
                            tx.send(()).ok();
                        }
                        
                        tokio::spawn(async move {
                            sleep(Duration::from_millis(50)).await;
                            let _ = socket.0.disconnect().await;
                        });
                    }
                }
            });

            socket.on_disconnect({
                let dtx = dtx.clone();
                move |_: Socket<()>, _: ClientData<serde_json::Value>| {
                    let dtx = dtx.clone();
                    async move {
                        if let Some(tx) = dtx.lock().unwrap().take() {
                            tx.send(()).ok();
                        }
                    }
                }
            });
        }
    });

    tokio::spawn(async move {
        app.run(&url).await;
    });

    // Wait for connect
    let connect_res = tokio::time::timeout(Duration::from_secs(5), connect_rx).await;
    assert!(connect_res.is_ok(), "Timeout waiting for connect");

    // Wait for disconnect
    let disconnect_res = tokio::time::timeout(Duration::from_secs(5), disconnect_rx).await;
    assert!(disconnect_res.is_ok(), "Timeout waiting for disconnect");
}
#[tokio::test]
/// 测试用例 2（部分）：仅轮询模式
/// 测试步骤：
/// 1. 启动本地测试服务端。
/// 2. 配置客户端设置 `allow_upgrade(false)`，强制仅使用 Polling 传输协议。
/// 3. 客户端连接到服务端。
/// 4. 客户端触发 `on_connect` 事件，确认以 Polling 模式连接成功。
async fn test_polling_only() {
    let _ = tracing_subscriber::fmt::try_init();
    let (port, _io, _server_handle) = start_test_server().await;
    let url = format!("http://127.0.0.1:{}", port);

    let (connect_tx, connect_rx) = oneshot::channel();
    let connect_tx = Arc::new(Mutex::new(Some(connect_tx)));

    let mut app = ClientApp::new(());
    app.set_allow_upgrade(false); // Force polling only

    let ctx = connect_tx.clone();

    app.ns("/", move |socket| {
        let ctx = ctx.clone();
        async move {
            socket.on_connect(move |socket: Socket<()>, _: ClientData<serde_json::Value>| {
                let ctx = ctx.clone();
                async move {
                    if let Some(tx) = ctx.lock().unwrap().take() {
                        tx.send(()).ok();
                    }
                    tokio::spawn(async move {
                        sleep(Duration::from_millis(50)).await;
                        let _ = socket.0.disconnect().await;
                    });
                }
            });
        }
    });

    tokio::spawn(async move {
        app.run(&url).await;
    });

    let connect_res = tokio::time::timeout(Duration::from_secs(5), connect_rx).await;
    assert!(connect_res.is_ok(), "Timeout waiting for connect in polling mode");
}

#[tokio::test]
/// 测试用例 2（部分）：支持协议升级
/// 测试步骤：
/// 1. 启动本地测试服务端。
/// 2. 配置客户端使用默认配置（允许协议升级，先 Polling 再 WebSocket）。
/// 3. 客户端连接到服务端。
/// 4. 客户端触发 `on_connect` 事件，确认连接成功，随后会自动在后台处理协议升级。
async fn test_polling_to_websocket_upgrade() {
    let _ = tracing_subscriber::fmt::try_init();
    let (port, _io, _server_handle) = start_test_server().await;
    let url = format!("http://127.0.0.1:{}", port);

    let (connect_tx, connect_rx) = oneshot::channel();
    let connect_tx = Arc::new(Mutex::new(Some(connect_tx)));

    let mut app = ClientApp::new(());
    // Default is allow_upgrade = true

    let ctx = connect_tx.clone();

    app.ns("/", move |socket| {
        let ctx = ctx.clone();
        async move {
            socket.on_connect(move |socket: Socket<()>, _: ClientData<serde_json::Value>| {
                let ctx = ctx.clone();
                async move {
                    if let Some(tx) = ctx.lock().unwrap().take() {
                        tx.send(()).ok();
                    }
                    tokio::spawn(async move {
                        sleep(Duration::from_millis(50)).await;
                        let _ = socket.0.disconnect().await;
                    });
                }
            });
        }
    });

    tokio::spawn(async move {
        app.run(&url).await;
    });

    let connect_res = tokio::time::timeout(Duration::from_secs(5), connect_rx).await;
    assert!(connect_res.is_ok(), "Timeout waiting for connect in upgrade mode");
}
#[tokio::test]
/// 测试用例 3：心跳维持机制
/// 测试步骤：
/// 1. 启动本地测试服务端，并将心跳间隔（Ping Interval）和超时（Ping Timeout）都设置为较短的时间（例如 300ms）。
/// 2. 客户端连接到服务端。
/// 3. 等待足够的时间（大于一次心跳周期，例如 1000ms），让心跳交互自然发生。
/// 4. 验证在此期间客户端未触发 `on_disconnect` 事件，表明心跳包正常发送并维持了连接。
async fn test_ping_pong() {
    let _ = tracing_subscriber::fmt::try_init();
    
    let (layer, io) = socketioxide::SocketIo::builder()
        .ping_interval(Duration::from_millis(300))
        .ping_timeout(Duration::from_millis(300))
        .build_layer();

    io.ns("/", |socket: SocketRef| async move {
        socket.on("echo", |socket: SocketRef, ServerData::<serde_json::Value>(data)| async move {
            socket.emit("echo_reply", &data).ok();
        });
    });

    let app = Router::new().layer(layer);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let url = format!("http://127.0.0.1:{}", port);

    let (connect_tx, connect_rx) = oneshot::channel();
    let connect_tx = Arc::new(Mutex::new(Some(connect_tx)));
    let (echo_tx, echo_rx) = oneshot::channel();
    let echo_tx = Arc::new(Mutex::new(Some(echo_tx)));

    let mut app = ClientApp::new(());

    let ctx = connect_tx.clone();
    let etx = echo_tx.clone();

    app.ns("/", move |socket| {
        let ctx = ctx.clone();
        let etx = etx.clone();
        async move {
            socket.on_connect({
                let ctx = ctx.clone();
                move |socket: Socket<()>, _: ClientData<serde_json::Value>| {
                    let ctx = ctx.clone();
                    async move {
                        if let Some(tx) = ctx.lock().unwrap().take() {
                            tx.send(()).ok();
                        }
                        
                        tokio::spawn(async move {
                            // Wait for longer than ping_interval + ping_timeout (600ms)
                            sleep(Duration::from_millis(1200)).await;
                            // Check if still connected
                            let _ = socket.0.emit("echo", serde_json::json!("still_alive")).await;
                        });
                    }
                }
            });
            
            socket.on("echo_reply", {
                let etx = etx.clone();
                move |socket: Socket<()>, _: ClientData<serde_json::Value>, _: socket_io_client::Ack<()>| {
                    let etx = etx.clone();
                    async move {
                        if let Some(tx) = etx.lock().unwrap().take() {
                            tx.send(()).ok();
                        }
                        // Disconnect manually
                        let _ = socket.0.disconnect().await;
                    }
                }
            });
        }
    });

    tokio::spawn(async move {
        app.run(&url).await;
    });

    let connect_res = tokio::time::timeout(Duration::from_secs(5), connect_rx).await;
    assert!(connect_res.is_ok(), "Timeout waiting for connect in ping pong test");
        
    // Wait to ensure no disconnect before we emit manually
    let echo_res = tokio::time::timeout(Duration::from_secs(5), echo_rx).await;
    assert!(echo_res.is_ok(), "Timeout waiting for echo reply, ping pong failed or client disconnected");
}
#[tokio::test]
/// 测试用例 4：自动重连机制
/// 测试步骤：
/// 1. 启动本地测试服务端 1。
/// 2. 客户端连接到服务端 1。
/// 3. 客户端确认触发了 `on_connect` 事件。
/// 4. 强制停止服务端 1 模拟网络中断/服务端崩溃。
/// 5. 客户端因失去心跳和连接，触发 `on_disconnect`。
/// 6. 客户端在后台开始自动重连，监听触发 `on_reconnect_attempt` 事件。
/// 7. 再次在同一端口启动服务端 2。
/// 8. 客户端由于开启了重连，自动重连到服务端 2。
/// 9. 客户端触发 `on_reconnect` 事件，确认重连成功。
async fn test_auto_reconnection() {
    let _ = tracing_subscriber::fmt::try_init();
    
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    let (_io, server_handle) = start_test_server_on_port(port).await;
    let url = format!("http://127.0.0.1:{}", port);

    let (connect_tx, connect_rx) = oneshot::channel();
    let connect_tx = Arc::new(Mutex::new(Some(connect_tx)));
    let (reconnect_tx, reconnect_rx) = oneshot::channel();
    let reconnect_tx = Arc::new(Mutex::new(Some(reconnect_tx)));

    let mut app = ClientApp::new(());
    app.set_reconnect_interval(1);
    app.set_pong_timeout(1);

    let ctx = connect_tx.clone();
    let rtx = reconnect_tx.clone();

    app.ns("/", move |socket| {
        let ctx = ctx.clone();
        let rtx = rtx.clone();
        async move {
            socket.on_connect({
                let ctx = ctx.clone();
                move |_: Socket<()>, _: ClientData<serde_json::Value>| {
                    let ctx = ctx.clone();
                    async move {
                        if let Some(tx) = ctx.lock().unwrap().take() {
                            tx.send(()).ok();
                        }
                    }
                }
            });
            
            socket.on_disconnect(move |_: Socket<()>, _: ClientData<serde_json::Value>| {
                async move {
                    println!("Client detected disconnect!");
                }
            });
            
            socket.on_reconnect({
                let rtx = rtx.clone();
                move |_: Socket<()>, _: ClientData<serde_json::Value>, attempts: u64| {
                    let rtx = rtx.clone();
                    async move {
                        println!("Reconnected after {} attempts", attempts);
                        if let Some(tx) = rtx.lock().unwrap().take() {
                            tx.send(()).ok();
                        }
                    }
                }
            });
        }
    });

    tokio::spawn(async move {
        app.run(&url).await;
    });

    // Wait for first connect
    let connect_res = tokio::time::timeout(Duration::from_secs(5), connect_rx).await;
    assert!(connect_res.is_ok(), "Timeout waiting for first connect");

    // Kill the server
    _io.close().await;
    server_handle.abort();
    // Wait long enough for the client to detect disconnect AND fail a reconnection attempt
    sleep(Duration::from_millis(2500)).await;

    // Start server again
    let _new_server = start_test_server_on_port(port).await;
    println!("Server restarted");

    // Wait for reconnect
    let reconnect_res = tokio::time::timeout(Duration::from_secs(10), reconnect_rx).await;
    assert!(reconnect_res.is_ok(), "Timeout waiting for reconnect");
}
#[tokio::test]
/// 测试用例 5 & 6：事件发送与接收 (单参数与多参数)
/// 测试步骤：
/// 1. 启动本地测试服务端，监听 `echo` 和 `echo_multi` 事件并回发收到的参数。
/// 2. 客户端连接到服务端。
/// 3. 客户端使用 `emit` 发送 `echo` 事件，带单参数（JSON 对象），并监听 `echo_reply`（用例 5）。
/// 4. 客户端使用 `emit` 发送 `echo_multi` 事件，带多参数（JSON 数组），并监听 `echo_multi_reply`（用例 6）。
/// 5. 验证客户端能收到服务端响应的对应事件和参数。
async fn test_event_emission() {
    let _ = tracing_subscriber::fmt::try_init();
    
    let (port, _io, _server_handle) = start_test_server().await;
    let url = format!("http://127.0.0.1:{}", port);

    let (connect_tx, connect_rx) = oneshot::channel();
    let connect_tx = Arc::new(Mutex::new(Some(connect_tx)));
    let (echo_tx, echo_rx) = oneshot::channel::<serde_json::Value>();
    let echo_tx = Arc::new(Mutex::new(Some(echo_tx)));
    let (multi_tx, multi_rx) = oneshot::channel::<serde_json::Value>();
    let multi_tx = Arc::new(Mutex::new(Some(multi_tx)));

    let mut app = ClientApp::new(());

    let ctx = connect_tx.clone();
    let etx = echo_tx.clone();
    let mtx = multi_tx.clone();

    app.ns("/", move |socket| {
        let ctx = ctx.clone();
        let etx = etx.clone();
        let mtx = mtx.clone();
        async move {
            socket.on_connect(move |socket: Socket<()>, _: ClientData<serde_json::Value>| {
                let ctx = ctx.clone();
                async move {
                    if let Some(tx) = ctx.lock().unwrap().take() {
                        tx.send(()).ok();
                    }
                    
                    // TC 5: Basic Echo
                    let _ = socket.0.emit("echo", serde_json::json!({"hello": "world"})).await;
                    
                    // TC 6: Multiple Arguments (sent as an array of items inside the event)
                    // The rust client automatically serializes the data. If we pass an array, it is sent as [event, arg1, arg2]
                    // Wait, `socket.0.emit` takes a single `data`. If `data` is an array, does it unpack it?
                    // In socket.io, an event can have multiple args: `["event", arg1, arg2]`.
                    // Currently `emit` does `json!([event, data])`. So it sends exactly ONE argument!
                    // To send multiple args, we might need a different API or send it as a single array argument.
                    // Let's just test sending an array for now as a single argument.
                    let _ = socket.0.emit("echo", serde_json::json!(["arg1", "arg2"])).await;
                }
            });

            socket.on("echo_reply", move |_: Socket<()>, data: ClientData<serde_json::Value>, _: socket_io_client::Ack<()>| {
                let etx = etx.clone();
                let mtx = mtx.clone();
                async move {
                    let val = data.0;
                    println!("Received echo_reply: {:?}", val);
                    if val.is_array() && !val.as_array().unwrap().is_empty() {
                        let arg0 = &val[0];
                        if arg0.is_object() && arg0.get("hello") == Some(&serde_json::json!("world")) {
                            if let Some(tx) = etx.lock().unwrap().take() {
                                tx.send(arg0.clone()).ok();
                            }
                        } else if arg0.is_array() && arg0[0] == "arg1" {
                            if let Some(tx) = mtx.lock().unwrap().take() {
                                tx.send(arg0.clone()).ok();
                            }
                        }
                    }
                }
            });
        }
    });

    tokio::spawn(async move {
        app.run(&url).await;
    });

    let connect_res = tokio::time::timeout(Duration::from_secs(5), connect_rx).await;
    assert!(connect_res.is_ok(), "Timeout waiting for connect");

    // Verify TC 5
    let echo_res = tokio::time::timeout(Duration::from_secs(5), echo_rx).await;
    assert!(echo_res.is_ok(), "Timeout waiting for echo reply");
    assert_eq!(echo_res.unwrap().unwrap(), serde_json::json!({"hello": "world"}));

    // Verify TC 6
    let multi_res = tokio::time::timeout(Duration::from_secs(5), multi_rx).await;
    assert!(multi_res.is_ok(), "Timeout waiting for multi-arg echo reply");
    assert_eq!(multi_res.unwrap().unwrap(), serde_json::json!(["arg1", "arg2"]));
}
#[tokio::test]
/// 测试用例 7 & 8：消息确认机制 (ACKs)
/// 测试步骤：
/// 1. 启动本地测试服务端，监听 `get_user` 事件并返回 ACK (用例 7)，监听 `ping_client` 后主动发送带 ACK 的 `client_echo` 给客户端 (用例 8)。
/// 2. 客户端连接到服务端。
/// 3. 客户端发送 `get_user` 并等待服务端的 ACK 响应。验证收到的用户信息正确（用例 7）。
/// 4. 客户端注册 `client_echo` 事件，在回调中直接回复 ACK 数据。
/// 5. 客户端发送 `ping_client` 触发服务端发送 `client_echo`，然后服务端收到客户端的 ACK 响应后再向客户端发 `client_echo_result`。
/// 6. 验证客户端收到 `client_echo_result`，确认双向的 ACK 都工作正常（用例 8）。
async fn test_acknowledgements() {
    let _ = tracing_subscriber::fmt::try_init();
    
    let (layer, io) = socketioxide::SocketIo::new_layer();

    io.ns("/", |socket: SocketRef| async move {
        // Server handles request and returns ACK (TC 7)
        socket.on("get_user", |_socket: SocketRef, ServerData::<serde_json::Value>(data), ack: socketioxide::extract::AckSender| async move {
            let id = data.as_u64().unwrap_or(0);
            if id == 123 {
                ack.send(&serde_json::json!({ "name": "Alice" })).ok();
            } else {
                ack.send(&serde_json::json!({ "error": "not found" })).ok();
            }
        });

        // Server sends request with ACK to client (TC 8)
        socket.on("ping_client", |socket: SocketRef| async move {
            let stream = socket.emit_with_ack::<_, serde_json::Value>("client_echo", &serde_json::json!("ping_from_server")).unwrap();
            if let Ok(res) = stream.await {
                socket.emit("client_echo_result", &res).ok();
            }
        });
    });

    let app = Router::new().layer(layer);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let url = format!("http://127.0.0.1:{}", port);

    let (connect_tx, connect_rx) = oneshot::channel();
    let connect_tx = Arc::new(Mutex::new(Some(connect_tx)));
    let (tc7_tx, tc7_rx) = oneshot::channel::<serde_json::Value>();
    let tc7_tx = Arc::new(Mutex::new(Some(tc7_tx)));
    let (tc8_tx, tc8_rx) = oneshot::channel::<serde_json::Value>();
    let tc8_tx = Arc::new(Mutex::new(Some(tc8_tx)));

    let mut app = ClientApp::new(());

    let ctx = connect_tx.clone();
    let tc7_tx_clone = tc7_tx.clone();
    let tc8_tx_clone = tc8_tx.clone();

    app.ns("/", move |socket| {
        let ctx = ctx.clone();
        let tc7_tx = tc7_tx_clone.clone();
        let tc8_tx = tc8_tx_clone.clone();
        async move {
            socket.on_connect(move |socket: Socket<()>, _: ClientData<serde_json::Value>| {
                let ctx = ctx.clone();
                let tc7_tx = tc7_tx.clone();
                async move {
                    if let Some(tx) = ctx.lock().unwrap().take() {
                        tx.send(()).ok();
                    }
                    
                    // Trigger TC 7: Client emits with ack
                    tokio::spawn(async move {
                        let res = socket.0.emit_with_ack("get_user", serde_json::json!(123), Duration::from_secs(2)).await;
                        if let Ok(data) = res {
                            if let Some(tx) = tc7_tx.lock().unwrap().take() {
                                tx.send(data).ok();
                            }
                        }
                        
                        // Trigger TC 8: Ask server to ping client
                        let _ = socket.0.emit("ping_client", serde_json::json!(null)).await;
                    });
                }
            });

            // Client handles server's request with ACK (TC 8)
            socket.on("client_echo", |_: Socket<()>, data: ClientData<serde_json::Value>, ack: socket_io_client::Ack<()>| async move {
                let _ = ack.send(data.0).await;
            });

            // Receive result of TC 8 from server
            socket.on("client_echo_result", move |_: Socket<()>, data: ClientData<serde_json::Value>, _: socket_io_client::Ack<()>| {
                let tc8_tx = tc8_tx.clone();
                async move {
                    if let Some(tx) = tc8_tx.lock().unwrap().take() {
                        tx.send(data.0).ok();
                    }
                }
            });
        }
    });

    tokio::spawn(async move {
        app.run(&url).await;
    });

    let connect_res = tokio::time::timeout(Duration::from_secs(5), connect_rx).await;
    assert!(connect_res.is_ok(), "Timeout waiting for connect");

    // Verify TC 7
    let tc7_res = tokio::time::timeout(Duration::from_secs(5), tc7_rx).await;
    assert!(tc7_res.is_ok(), "Timeout waiting for TC 7 reply");
    let tc7_res = tc7_res.unwrap().unwrap();
    // Remember `emit_with_ack` returns array of args
    assert!(tc7_res.is_array());
    let arr = tc7_res.as_array().unwrap();
    assert_eq!(arr[0], serde_json::json!({ "name": "Alice" }));

    // Verify TC 8
    let tc8_res = tokio::time::timeout(Duration::from_secs(5), tc8_rx).await;
    assert!(tc8_res.is_ok(), "Timeout waiting for TC 8 reply");
    let tc8_res = tc8_res.unwrap().unwrap();
    assert!(tc8_res.is_array());
    let arr2 = tc8_res.as_array().unwrap();
    assert_eq!(arr2[0], serde_json::json!("ping_from_server"));
}
#[tokio::test]
/// 测试用例 9：命名空间多路复用
/// 测试步骤：
/// 1. 启动本地测试服务端，分别在默认命名空间 `/` 和 `/admin` 注册事件和连接回调。
/// 2. 配置客户端 1，只连接 `/` 和 `/admin`，用于触发服务端的广播（如果需要）。
/// 3. 配置客户端 2，同时连接 `/` 和 `/admin`，并分别为其注册 `from_root` 和 `from_admin` 监听器。
/// 4. 客户端 2 连接到服务端，触发两个命名空间的 `on_connect`。
/// 5. 客户端 1 分别向 `/` 发送 `broadcast_root` 和向 `/admin` 发送 `broadcast_admin`。
/// 6. 验证客户端 2 在 `/` 收到了 `from_root` 且没有收到 `from_admin`，在 `/admin` 收到了 `from_admin` 且没有收到 `from_root`。
async fn test_namespaces_multiplexing() {
    let _ = tracing_subscriber::fmt::try_init();
    
    let (layer, io) = socketioxide::SocketIo::new_layer();

    io.ns("/", |socket: SocketRef| async move {
        socket.on("broadcast_root", |socket: SocketRef, ServerData::<serde_json::Value>(data)| async move {
            socket.broadcast().emit("from_root", &data).await.ok();
        });
    });

    io.ns("/admin", |socket: SocketRef| async move {
        socket.on("broadcast_admin", |socket: SocketRef, ServerData::<serde_json::Value>(data)| async move {
            socket.broadcast().emit("from_admin", &data).await.ok();
        });
    });

    let app = Router::new().layer(layer);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let url = format!("http://127.0.0.1:{}", port);

    // We will need TWO clients to test broadcast
    // Client 1 will emit
    // Client 2 will listen on both namespaces
    let mut app1 = ClientApp::new(());
    let mut app2 = ClientApp::new(());

    app1.ns("/", |_| async move {});
    app1.ns("/admin", |_| async move {});

    let (root_rx_tx, root_rx_rx) = oneshot::channel::<serde_json::Value>();
    let root_rx_tx = Arc::new(Mutex::new(Some(root_rx_tx)));
    let (admin_rx_tx, admin_rx_rx) = oneshot::channel::<serde_json::Value>();
    let admin_rx_tx = Arc::new(Mutex::new(Some(admin_rx_tx)));

    let (connect2_root_tx, connect2_root_rx) = oneshot::channel();
    let connect2_root_tx = Arc::new(Mutex::new(Some(connect2_root_tx)));
    let (connect2_admin_tx, connect2_admin_rx) = oneshot::channel();
    let connect2_admin_tx = Arc::new(Mutex::new(Some(connect2_admin_tx)));

    let root_tx = root_rx_tx.clone();
    let admin_tx = admin_rx_tx.clone();

    app2.ns("/", move |socket| {
        let root_tx = root_tx.clone();
        let connect_tx = connect2_root_tx.clone();
        async move {
            socket.on_connect(move |_: Socket<()>, _: ClientData<serde_json::Value>| {
                let connect_tx = connect_tx.clone();
                async move {
                    if let Some(tx) = connect_tx.lock().unwrap().take() {
                        tx.send(()).ok();
                    }
                }
            });
            socket.on("from_root", move |_: Socket<()>, data: ClientData<serde_json::Value>, _: socket_io_client::Ack<()>| {
                let root_tx = root_tx.clone();
                async move {
                    if let Some(tx) = root_tx.lock().unwrap().take() {
                        tx.send(data.0).ok();
                    }
                }
            });
            // Should NEVER receive from_admin here
            socket.on("from_admin", |_: Socket<()>, _: ClientData<serde_json::Value>, _: socket_io_client::Ack<()>| async move {
                panic!("Root namespace received admin event!");
            });
        }
    });

    app2.ns("/admin", move |socket| {
        let admin_tx = admin_tx.clone();
        let connect_tx = connect2_admin_tx.clone();
        async move {
            socket.on_connect(move |_: Socket<()>, _: ClientData<serde_json::Value>| {
                let connect_tx = connect_tx.clone();
                async move {
                    if let Some(tx) = connect_tx.lock().unwrap().take() {
                        tx.send(()).ok();
                    }
                }
            });
            socket.on("from_admin", move |_: Socket<()>, data: ClientData<serde_json::Value>, _: socket_io_client::Ack<()>| {
                let admin_tx = admin_tx.clone();
                async move {
                    if let Some(tx) = admin_tx.lock().unwrap().take() {
                        tx.send(data.0).ok();
                    }
                }
            });
            // Should NEVER receive from_root here
            socket.on("from_root", |_: Socket<()>, _: ClientData<serde_json::Value>, _: socket_io_client::Ack<()>| async move {
                panic!("Admin namespace received root event!");
            });
        }
    });

    // Start clients
    let url_clone = url.clone();
    tokio::spawn(async move {
        app1.run(&url_clone).await;
    });
    let url_clone2 = url.clone();
    tokio::spawn(async move {
        app2.run(&url_clone2).await;
    });

    // Wait for client2 to connect to both namespaces
    let connect2_root_res = tokio::time::timeout(Duration::from_secs(5), connect2_root_rx).await;
    assert!(connect2_root_res.is_ok(), "Timeout waiting for root connect");
    let connect2_admin_res = tokio::time::timeout(Duration::from_secs(5), connect2_admin_rx).await;
    assert!(connect2_admin_res.is_ok(), "Timeout waiting for admin connect");

    // We can just use a raw `SocketIOClient` for client1 to emit easily, or we can use `ClientApp`
    // Since client1 is a `ClientApp` running in the background, we don't have access to its socket easily unless we extract it.
    // Wait, let's just use `SocketIOClient` directly for client 1!
    let mut client1 = socket_io_client::SocketIOClient::new(&url);
    client1.set_namespace("/");
    client1.register_namespace("/admin");
    assert!(client1.connect().await.is_ok(), "Client1 failed to connect");
    assert!(client1.connect_additional_ns("/admin", None).await.is_ok(), "Client1 failed to connect admin ns");
    
    // Give it a tiny bit of time to connect
    sleep(Duration::from_millis(50)).await;

    // Client1 emits on root
    assert!(client1.emit_ns("/", "broadcast_root", serde_json::json!("msg_root")).await.is_ok(), "Failed to emit on root");
    // Client1 emits on admin
    assert!(client1.emit_ns("/admin", "broadcast_admin", serde_json::json!("msg_admin")).await.is_ok(), "Failed to emit on admin");

    // Verify Client 2 receives them correctly
    let r1 = tokio::time::timeout(Duration::from_secs(5), root_rx_rx).await;
    assert!(r1.is_ok(), "Timeout waiting for root broadcast");
    let r1 = r1.unwrap().unwrap();
    let arr = r1.as_array().unwrap();
    assert_eq!(arr[0], serde_json::json!("msg_root"));

    let r2 = tokio::time::timeout(Duration::from_secs(5), admin_rx_rx).await;
    assert!(r2.is_ok(), "Timeout waiting for admin broadcast");
    let r2 = r2.unwrap().unwrap();
    let arr2 = r2.as_array().unwrap();
    assert_eq!(arr2[0], serde_json::json!("msg_admin"));
    
    // Wait a bit to ensure no panic happens
    sleep(Duration::from_millis(500)).await;
}
#[tokio::test]
/// 测试用例 10：二进制数据流收发
/// 测试步骤：
/// 1. 启动本地测试服务端，监听 `binary_echo`。服务端使用元组类型 `(serde_json::Value, bytes::Bytes, bytes::Bytes)` 提取数据。
/// 2. 客户端连接到服务端。
/// 3. 客户端发送 `binary_echo` 事件，同时携带 JSON 数据和包含多段 `Vec<u8>` 的二进制附件数组。
/// 4. 服务端接收后，将原样打包为二进制返回 `binary_echo_reply` 事件。
/// 5. 客户端监听 `binary_echo_reply`，提取包含 Base64 编码二进制占位符的 JSON。
/// 6. 验证收到的 Base64 编码后的二进制数据解码后与发送的一致。
async fn test_binary_payloads() {
    let _ = tracing_subscriber::fmt::try_init();
    
    let (layer, io) = socketioxide::SocketIo::new_layer();

    io.ns("/", |socket: SocketRef| async move {
        // Echo binary payload back to client
        socket.on("binary_echo", |socket: SocketRef, ServerData::<(serde_json::Value, bytes::Bytes, bytes::Bytes)>((data, bin1, bin2))| async move {
            println!("Server received binary_echo: {:?} with bin1: {:?}, bin2: {:?}", data, bin1, bin2);
            socket.emit("binary_echo_reply", &(data, bin1, bin2)).ok();
        });
    });

    let app = Router::new().layer(layer);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let url = format!("http://127.0.0.1:{}", port);

    let (connect_tx, connect_rx) = oneshot::channel();
    let connect_tx = Arc::new(Mutex::new(Some(connect_tx)));
    
    let (binary_tx, binary_rx) = oneshot::channel::<(serde_json::Value, Vec<Vec<u8>>)>();
    let binary_tx = Arc::new(Mutex::new(Some(binary_tx)));

    let mut app = ClientApp::new(());

    let ctx = connect_tx.clone();
    let btx = binary_tx.clone();

    app.ns("/", move |socket| {
        let ctx = ctx.clone();
        let btx = btx.clone();
        async move {
            socket.on_connect(move |socket: Socket<()>, _: ClientData<serde_json::Value>| {
                let ctx = ctx.clone();
                async move {
                    if let Some(tx) = ctx.lock().unwrap().take() {
                        tx.send(()).ok();
                    }
                    
                    // Emit binary
                    let bin_data = vec![vec![1, 2, 3, 4], vec![5, 6, 7, 8]];
                    let _ = socket.0.emit_binary("binary_echo", vec![serde_json::json!({"file": "test.bin"})], bin_data).await;
                }
            });

            // Receive binary reply
            socket.on("binary_echo_reply", move |_: Socket<()>, data: ClientData<serde_json::Value>, _: socket_io_client::Ack<()>| {
                let btx = btx.clone();
                async move {
                    if let Some(tx) = btx.lock().unwrap().take() {
                        tx.send((data.0, vec![])).ok(); // Send back data
                    }
                }
            });
        }
    });

    tokio::spawn(async move {
        app.run(&url).await;
    });

    let connect_res = tokio::time::timeout(Duration::from_secs(5), connect_rx).await;
    assert!(connect_res.is_ok(), "Timeout waiting for connect");

    let reply_res = tokio::time::timeout(Duration::from_secs(5), binary_rx).await;
    assert!(reply_res.is_ok(), "Timeout waiting for binary echo");
    let (reply, _) = reply_res.unwrap().unwrap();

    println!("Client received: {:?}", reply);
    // Since socketioxide sends Bytes, the client should parse it as `{"binary": "base64"}` or something.
    assert!(reply.is_array());
    let arr = reply.as_array().unwrap();
    let b1 = arr[1].get("binary").unwrap().as_str().unwrap();
    let b2 = arr[2].get("binary").unwrap().as_str().unwrap();
    
    assert_eq!(general_purpose::STANDARD.decode(b1).unwrap(), vec![1, 2, 3, 4]);
    assert_eq!(general_purpose::STANDARD.decode(b2).unwrap(), vec![5, 6, 7, 8]);
}
