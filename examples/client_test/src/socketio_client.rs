//! Socket.IO 客户端核心逻辑
//!
//! 包含客户端状态管理、命名空间注册、事件监听与发送等功能。

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tokio::fs;
use tokio::sync::Mutex;
use tracing::{info, warn};

use socket_io_client::{
    ClientApp, ClientSocketRef, Data as CData, Socket as CSocket, State as CState,
};

const UPLOAD_CHUNK_SIZE: usize = 256 * 1024;

/// 设备类型枚举
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub enum DeviceType {
    #[default]
    WebApp,
    SIOServer,
    SIOClient,
}

/// 设备注册数据结构
#[derive(Debug, Serialize, Deserialize)]
pub struct RegisterDeviceData {
    /// 设备类型
    pub device_type: DeviceType,
    /// 设备唯一标识
    pub device_id: String,
}

/// 客户端全局状态
#[derive(Clone)]
pub struct AppState {
    /// 内部状态，使用 Arc<Mutex> 保证并发安全
    pub status: Arc<Mutex<InnerStatus>>,
}

/// 客户端内部状态数据
pub struct InnerStatus {
    /// 是否正在发送心跳 tick
    pub is_ticking: bool,
    /// 已经发送的 tick 次数
    pub ticks: u64,
}

/// 启动 Socket.IO 客户端主循环
pub async fn start_client() {
    let state = AppState {
        status: Arc::new(Mutex::new(InnerStatus {
            is_ticking: false,
            ticks: 0,
        })),
    };

    let mut app = ClientApp::new(state);
    // 配置重连间隔和超时时间
    app.set_reconnect_interval(3);
    app.set_pong_timeout(8);

    register_root_ns(&mut app);
    register_admin_ns(&mut app);

    app.run("http://127.0.0.1:3000").await;
}

/// 注册根命名空间 `/` 的事件监听
fn register_root_ns(app: &mut ClientApp<AppState>) {
    app.ns("/", |socket: ClientSocketRef<AppState>| async move {
        socket.on_connect::<CSocket<AppState>, CState<AppState>, _, _>(
            |CSocket(s): CSocket<AppState>, CState(_): CState<AppState>| async move {
                info!("client: / connected, 最终使用协议: {:?}", s.transport());

                // 注册为 SIOClient
                let reg_data = RegisterDeviceData {
                    device_type: DeviceType::SIOClient,
                    device_id: "local_client_1".to_string(),
                };
                info!(?reg_data, "client: 准备发送 RegisterDevice 指令");
                if let Err(e) = s.emit("RegisterDevice", json!(reg_data)).await {
                    warn!(%e, "client: RegisterDevice 发送失败");
                } else {
                    info!("client: RegisterDevice 发送成功");
                }
                let _ = s.emit("echo", json!({"field1": "v1", "field2": 2})).await;
                let _ = s
                    .emit_args(
                        "echo_multi",
                        vec![json!("a"), json!({"b": 1}), json!(3)],
                    )
                    .await;
            },
        );

        socket.on_disconnect::<CSocket<AppState>, CState<AppState>, _, _>(
            |CSocket(_): CSocket<AppState>, CState(_): CState<AppState>| async move {
                warn!("client: / disconnected");
            },
        );

        socket.on::<CSocket<AppState>, CData<Value>, CState<AppState>, _, _>(
            "toggle_tick",
            |CSocket(_), CData(data), CState(state)| async move {
                let is_ticking = match &data {
                    Value::Array(arr) => arr.first().and_then(|v| v.as_bool()).unwrap_or(false),
                    Value::Bool(b) => *b,
                    _ => false,
                };
                info!(?data, is_ticking, "client: 收到 toggle_tick 指令, 参数为: {}", is_ticking);
                state.status.lock().await.is_ticking = is_ticking;
            },
        );

        socket.on::<CSocket<AppState>, CData<Value>, CState<AppState>, _, _>(
            "client_to_server_echo_json",
            |CSocket(s), CData(data), CState(_)| async move {
                info!(?data, "client: 收到 client_to_server_echo_json 指令，准备按 JSON 对象发送");
                if let Err(e) = s.emit("echo", &data).await {
                    warn!(%e, "client: 发送 echo 失败");
                }
            },
        );

        socket.on::<CSocket<AppState>, CData<Value>, CState<AppState>, _, _>(
            "client_to_server_echo_multi",
            |CSocket(s), CData(data), CState(_)| async move {
                info!(?data, "client: 收到 client_to_server_echo_multi 指令，准备按多参数发送");
                
                // 将 data 尝试解析为 JSON 数组字符串，如果本身就是数组则直接使用
                let args = match data {
                    Value::Array(arr) => arr,
                    Value::String(s) => {
                        match serde_json::from_str::<Vec<Value>>(&s) {
                            Ok(arr) => arr,
                            Err(_) => {
                                warn!("client_to_server_echo_multi: 期望接收 JSON 数组字符串，但解析失败。实际内容: {}", s);
                                return;
                            }
                        }
                    },
                    other => {
                        warn!("client_to_server_echo_multi: 期望接收 JSON 数组或数组格式的字符串，但收到: {:?}", other);
                        return;
                    },
                };

                if let Err(e) = s.emit_args("echo_multi", args).await {
                    warn!(%e, "client: 发送 echo_multi 失败");
                }
            },
        );

        socket.on::<CSocket<AppState>, CData<Value>, CState<AppState>, _, _>(
            "server_to_client_echo",
            |CSocket(_), CData(data), CState(_)| async move {
                info!(?data, "client: 收到 server_to_client_echo, 客户端与服务器端通信流程完成！");
            },
        );
        socket.on::<CSocket<AppState>, CData<Value>, CState<AppState>, _, _>(
            "echo_reply",
            |CSocket(_), CData(data), CState(_)| async move {
                info!(?data, "client: 收到 echo_reply");
            },
        );
        socket.on::<CSocket<AppState>, CData<Value>, CState<AppState>, _, _>(
            "echo_multi_reply",
            |CSocket(_), CData(data), CState(_)| async move {
                info!(?data, "client: 收到 echo_multi_reply");
            },
        );

        // 处理功能3：TC7 (客户端主动发 ACK 请求)
        socket.on::<CSocket<AppState>, CData<Value>, CState<AppState>, _, _>(
            "trigger_tc7",
            |CSocket(s), CData(data), CState(_)| async move {
                info!(?data, "client: 收到网页端 trigger_tc7 指令，准备向服务端发送带 ACK 的 get_user 请求");
                match s.emit_with_ack("get_user", json!(123), Duration::from_secs(3)).await {
                    Ok(res) => info!("client/get_user: ✅ 成功收到服务端的 ACK 响应数据: {}", res),
                    Err(e) => warn!("client/get_user: ❌ 等待服务端 ACK 超时或失败: {:?}", e),
                }
            },
        );

        socket.on::<CSocket<AppState>, CData<Value>, CState<AppState>, _, _>(
            "upload_test_zip",
            |CSocket(s), CData(data), CState(_)| async move {
                info!(?data, "client: 收到网页端 upload_test_zip 指令，准备读取本地 test.zip 并上传");
                let _ = s
                    .emit(
                        "client_upload_status",
                        json!({
                            "step": "start",
                            "message": "开始读取 client_test/test.zip",
                        }),
                    )
                    .await;

                let zip_path = format!("{}/test.zip", env!("CARGO_MANIFEST_DIR"));
                let zip_bytes = match fs::read(&zip_path).await {
                    Ok(bytes) => bytes,
                    Err(e) => {
                        warn!(path = %zip_path, error = %e, "client: 读取 test.zip 失败");
                        let _ = s
                            .emit(
                                "client_upload_status",
                                json!({
                                    "step": "read_failed",
                                    "message": format!("读取 test.zip 失败: {e}"),
                                    "path": zip_path,
                                }),
                            )
                            .await;
                        return;
                    }
                };

                let meta = json!({
                    "file_name": "test.zip",
                    "content_type": "application/zip",
                    "source_path": zip_path,
                    "size": zip_bytes.len(),
                });

                let upload_id = format!(
                    "upload-{}",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|duration| duration.as_millis())
                        .unwrap_or_default()
                );
                let total_chunks = zip_bytes.len().div_ceil(UPLOAD_CHUNK_SIZE);

                let _ = s
                    .emit(
                        "client_upload_status",
                        json!({
                            "step": "chunking",
                            "message": format!(
                                "test.zip 大小 {} 字节，按 {} 字节分为 {} 个分片上传",
                                zip_bytes.len(),
                                UPLOAD_CHUNK_SIZE,
                                total_chunks
                            ),
                            "upload_id": upload_id,
                        }),
                    )
                    .await;

                let file_name = meta
                    .get("file_name")
                    .and_then(Value::as_str)
                    .unwrap_or("test.zip")
                    .to_string();

                let mut send_error: Option<String> = None;

                for (chunk_index, chunk) in zip_bytes.chunks(UPLOAD_CHUNK_SIZE).enumerate() {
                    let chunk_meta = json!({
                        "upload_id": upload_id,
                        "file_name": file_name,
                        "content_type": "application/zip",
                        "total_size": zip_bytes.len(),
                        "chunk_index": chunk_index,
                        "total_chunks": total_chunks,
                        "chunk_size": chunk.len(),
                        "is_last_chunk": chunk_index + 1 == total_chunks,
                    });

                    if let Err(e) = s
                        .emit_binary_with_ack(
                            "upload_zip_chunk",
                            vec![chunk_meta],
                            vec![chunk.to_vec()],
                            Duration::from_secs(5),
                        )
                        .await
                    {
                        send_error = Some(e.to_string());
                        warn!(
                            error = %e,
                            chunk_index,
                            total_chunks,
                            "client: 上传 test.zip 分片失败"
                        );
                        break;
                    }

                    if chunk_index == 0 || chunk_index + 1 == total_chunks || chunk_index % 64 == 0 {
                        let _ = s
                            .emit(
                                "client_upload_status",
                                json!({
                                    "step": "progress",
                                    "message": format!(
                                        "test.zip 分片上传进度: {}/{}",
                                        chunk_index + 1,
                                        total_chunks
                                    ),
                                    "upload_id": upload_id,
                                }),
                            )
                            .await;
                    }
                }

                if let Some(error) = send_error {
                    let _ = s
                        .emit(
                            "client_upload_status",
                            json!({
                                "step": "send_failed",
                                "message": format!("上传 test.zip 失败: {error}"),
                                "upload_id": upload_id,
                            }),
                        )
                        .await;
                } else {
                    info!(
                        total_chunks,
                        chunk_size = UPLOAD_CHUNK_SIZE,
                        "client: test.zip 已按分片方式发送给服务端"
                    );
                    let _ = s
                        .emit(
                            "client_upload_status",
                            json!({
                                "step": "sent",
                                "message": format!("test.zip 已完成分片发送，共 {} 片", total_chunks),
                                "upload_id": upload_id,
                            }),
                        )
                        .await;
                }
            },
        );

        // 每 2 秒向 server 发送一次 tick
        socket.run_interval::<CSocket<AppState>, CState<AppState>, _, _>(
            "tick-loop",
            Duration::from_secs(2),
            0,
            |_name, _int, _count, CSocket(s), CState(state)| async move {
                let (n, should_tick) = {
                    let mut g = state.status.lock().await;
                    if g.is_ticking {
                        g.ticks = g.ticks.saturating_add(1);
                        (g.ticks, true)
                    } else {
                        (g.ticks, false)
                    }
                };
                if should_tick {
                    if let Err(e) = s.emit("tick", json!({ "n": n })).await {
                        warn!(%e, "client: tick 发送失败");
                    }
                }
            },
        );
    });
}

/// 注册 `/admin` 命名空间的事件监听
fn register_admin_ns(app: &mut ClientApp<AppState>) {
    app.ns("/admin", |socket: ClientSocketRef<AppState>| async move {
        socket.on_connect::<CSocket<AppState>, CState<AppState>, _, _>(
            |CSocket(_): CSocket<AppState>, CState(_): CState<AppState>| async move {
                info!("client: /admin connected");
            },
        );

        socket.on_disconnect::<CSocket<AppState>, CState<AppState>, _, _>(
            |CSocket(_): CSocket<AppState>, CState(_): CState<AppState>| async move {
                warn!("client: /admin disconnected");
            },
        );
    });
}
