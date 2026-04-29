use serde_json::{json, Value};
use socketioxide::layer::SocketIoLayer;
use socketioxide::{
    extract::{Data, SocketRef, State},
    SocketIo,
};
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tracing::{info, warn};
use crate::app::{CommonCommand, CommandStage, CommandCastMode, DeviceType, RegisterDeviceData};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone)]
pub struct RuntimeInfo {
    pub socket_ref: Option<SocketRef>,
    pub device_type: DeviceType,
    pub device_id: String,
}

#[derive(Debug, Clone)]
pub struct DeviceRuntimeInfo {
    pub device_runtime_info_map: Arc<Mutex<HashMap<socketioxide::socket::Sid, RuntimeInfo>>>,
}

#[derive(Debug, Clone)]
pub struct SocketAppState {
    pub device_runtime_info: DeviceRuntimeInfo,
}

/// 构建 socketioxide 的 Layer，并注册 `/` 与 `/admin` 命名空间。返回值同时暴露 `SocketIo`
/// 句柄，便于外部向浏览器端推送日志、状态等事件。
pub fn build_socketio_layer() -> (SocketIoLayer, SocketIo) {
    let state = SocketAppState {
        device_runtime_info: DeviceRuntimeInfo {
            device_runtime_info_map: Arc::new(Mutex::new(HashMap::new())),
        },
    };

    let (layer, io) = SocketIo::builder()
        .ping_interval(std::time::Duration::from_secs(20))
        .ping_timeout(std::time::Duration::from_secs(20))
        .max_payload(10 * 1024 * 1024) // 10MB just in case
        .with_state(state)
        .build_layer();

    info!("Server starting, registering as SIOServer...");

    io.ns("/", |socket: SocketRef| async move {
        info!(sid = %socket.id, "server: / connected");

        socket.on("RegisterDevice", |socket: SocketRef, Data::<Value>(raw_val), socket_app_state: State<SocketAppState>| async move {
            info!(sid = %socket.id, payload = %raw_val, "server/RegisterDevice: 收到原始 payload");
            let data: RegisterDeviceData = match serde_json::from_value(raw_val.clone()) {
                Ok(d) => d,
                Err(e) => {
                    warn!("RegisterDevice 解析失败: {}", e);
                    return;
                }
            };
            info!(sid = %socket.id, device_type = ?data.device_type, device_id = %data.device_id, "server/RegisterDevice: 设备注册");
            
            // 存入 DEVICE_MAP
            let map_arc = &socket_app_state.device_runtime_info.device_runtime_info_map;
            let mut map = map_arc.lock().unwrap();
            map.insert(socket.id, RuntimeInfo {
                socket_ref: Some(socket.clone()),
                device_type: data.device_type.clone(),
                device_id: data.device_id.clone(),
            });

            let room_name = format!("room_{:?}", data.device_type);
            let _ = socket.join(room_name);
        });

        socket.on("DealCommonCommand", |socket: SocketRef, Data::<Value>(raw_val), socket_app_state: State<SocketAppState>| async move {
            info!(sid = %socket.id, payload = %raw_val, "server/DealCommonCommand: 收到原始 payload");
            let cmd: CommonCommand = match serde_json::from_value(raw_val.clone()) {
                Ok(c) => c,
                Err(e) => {
                    warn!("DealCommonCommand 解析失败: {}", e);
                    return;
                }
            };
            info!(
                sid = %socket.id,
                command = %cmd.command,
                stage = ?cmd.stage,
                payload = %serde_json::to_string_pretty(&cmd).unwrap_or_else(|_| format!("{:?}", cmd)),
                "server/DealCommonCommand: 收到通用指令"
            );
            
            let (target_type, target_id, cast_mode) = match cmd.stage {
                CommandStage::Request => (&cmd.request_device_type, &cmd.request_device_id, &cmd.request_cast_mode),
                CommandStage::Response => (&cmd.response_device_type, &cmd.response_device_id, &cmd.response_cast_mode),
            };

            let payload = match serde_json::from_str::<Value>(&cmd.params) {
                Ok(v) => v,
                Err(_) => json!({ "data": cmd.params }),
            };

            let map_arc = &socket_app_state.device_runtime_info.device_runtime_info_map;
            let map = map_arc.lock().unwrap();

            if *cast_mode == CommandCastMode::Unicast {
                if let Some(target_info) = map.values().find(|info| &info.device_id == target_id) {
                    if let Some(target_sock) = &target_info.socket_ref {
                        info!("单播发送指令 {} 到 device_id: {}", cmd.command, target_id);
                        let _ = target_sock.emit(&cmd.command, &payload);
                    }
                } else {
                    warn!("未找到目标设备 device_id: {}", target_id);
                }
            } else {
                info!("广播发送指令 {} 到 device_type: {:?}", cmd.command, target_type);
                let room_name = format!("room_{:?}", target_type);
                let _ = socket.broadcast().within(room_name).emit(&cmd.command, &payload);
            }
        });



        // 处理客户端发来的 echo_to_server 事件，并回发给客户端
        socket.on("echo_to_server", |socket: SocketRef, Data::<Value>(data)| async move {
            info!(?data, "server/echo_to_server: 收到客户端消息，准备回发 server_to_client_echo");
            let _ = socket.emit("server_to_client_echo", &data);
        });

        // 原生 client 每 2 秒发来一次 tick；收到后把数值广播给所有浏览器连接
        socket.on("tick", |socket: SocketRef, Data::<Value>(data)| async move {
            let n = data.get("n").and_then(Value::as_u64).unwrap_or(0);
            info!(n, "server/tick: 收到心跳");
            let _ = socket
                .broadcast()
                .emit("status:ticks", &json!({ "ticks": n }))
                .await;
        });
        socket.on("echo", |socket: SocketRef, Data::<Value>(data)| async move {
            let _ = socket.emit("echo_reply", &data);
        });
        socket.on("echo_multi", |socket: SocketRef, Data::<Value>(data)| async move {
            let _ = socket.emit("echo_multi_reply", &data);
        });

        // 响应客户端发起的 ACK 请求（用例 7）
        socket.on("get_user", |_socket: SocketRef, Data::<Value>(data), ack: socketioxide::extract::AckSender| async move {
            info!(?data, "server/get_user: 收到客户端带 ACK 的请求");
            let id = data.as_u64().unwrap_or(0);
            if id == 123 {
                ack.send(&json!({ "name": "Alice", "msg": "这是服务端通过 ACK 返回的数据" })).ok();
            } else {
                ack.send(&json!({ "error": "not found" })).ok();
            }
        });

        socket.on("client_upload_status", |_socket: SocketRef, Data::<Value>(data)| async move {
            info!(payload = %data, "server/client_upload_status: 收到客户端上传状态");
        });

        socket.on("upload_zip_chunk", |socket: SocketRef, Data::<(Value, bytes::Bytes)>((meta, file_bytes)), ack: socketioxide::extract::AckSender| async move {
            let upload_id = meta
                .get("upload_id")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .unwrap_or("unknown-upload");
            let file_name = meta
                .get("file_name")
                .and_then(Value::as_str)
                .filter(|name| !name.is_empty())
                .unwrap_or("upload.bin");
            let upload_dir = format!("{}/uploads", env!("CARGO_MANIFEST_DIR"));
            let temp_path = format!("{upload_dir}/{upload_id}.part");
            let save_path = format!("{upload_dir}/{file_name}");
            let chunk_index = meta
                .get("chunk_index")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let total_chunks = meta
                .get("total_chunks")
                .and_then(Value::as_u64)
                .unwrap_or(1);
            let is_last_chunk = meta
                .get("is_last_chunk")
                .and_then(Value::as_bool)
                .unwrap_or(false);

            if let Err(e) = fs::create_dir_all(&upload_dir).await {
                warn!(path = %upload_dir, error = %e, "server/upload_zip_chunk: 创建上传目录失败");
                ack.send(&json!({"status": "error", "message": "Failed to create directory"})).ok();
                return;
            }

            if chunk_index == 0 {
                if let Err(e) = fs::remove_file(&temp_path).await {
                    if e.kind() != std::io::ErrorKind::NotFound {
                        warn!(path = %temp_path, error = %e, "server/upload_zip_chunk: 删除旧临时文件失败");
                    }
                }
            }

            let mut output = match fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&temp_path)
                .await
            {
                Ok(file) => file,
                Err(e) => {
                    warn!(path = %temp_path, error = %e, "server/upload_zip_chunk: 打开临时文件失败");
                    ack.send(&json!({"status": "error", "message": "Failed to open temp file"})).ok();
                    return;
                }
            };

            if let Err(e) = output.write_all(&file_bytes).await {
                warn!(path = %temp_path, error = %e, "server/upload_zip_chunk: 写入分片失败");
                ack.send(&json!({"status": "error", "message": "Failed to write chunk"})).ok();
                return;
            }

            if chunk_index == 0 || is_last_chunk || chunk_index % 64 == 0 {
                info!(
                    sid = %socket.id,
                    upload_id = %upload_id,
                    chunk_index,
                    total_chunks,
                    chunk_size = file_bytes.len(),
                    "server/upload_zip_chunk: 已接收文件分片"
                );
            }

            if is_last_chunk {
                if let Err(e) = output.flush().await {
                    warn!(path = %temp_path, error = %e, "server/upload_zip_chunk: 刷新临时文件失败");
                    ack.send(&json!({"status": "error", "message": "Failed to flush temp file"})).ok();
                    return;
                }
                drop(output);

                if let Err(e) = fs::rename(&temp_path, &save_path).await {
                    warn!(
                        temp_path = %temp_path,
                        save_path = %save_path,
                        error = %e,
                        "server/upload_zip_chunk: 完成分片上传后重命名失败"
                    );
                    ack.send(&json!({"status": "error", "message": "Failed to rename file"})).ok();
                    return;
                }

                let saved_size = fs::metadata(&save_path)
                    .await
                    .map(|metadata| metadata.len())
                    .unwrap_or_default();

                info!(
                    sid = %socket.id,
                    upload_id = %upload_id,
                    path = %save_path,
                    file_name = %file_name,
                    size = saved_size,
                    total_chunks,
                    "server/upload_zip_chunk: 已接收并保存完整二进制文件"
                );

                let _ = socket.emit("upload_zip_file_saved", &json!({
                    "upload_id": upload_id,
                    "file_name": file_name,
                    "saved_path": save_path,
                    "size": saved_size,
                    "total_chunks": total_chunks,
                }));
            }

            ack.send(&json!({"status": "ok"})).ok();
        });

        socket.on_disconnect(|socket: SocketRef, socket_app_state: State<SocketAppState>| async move {
            warn!(sid = %socket.id, "server: / disconnected");
            let map_arc = &socket_app_state.device_runtime_info.device_runtime_info_map;
            let mut map = map_arc.lock().unwrap();
            map.remove(&socket.id);
        });
    });

    io.ns("/admin", |socket: SocketRef| async move {
        info!(sid = %socket.id, "server: /admin connected");



        socket.on_disconnect(|socket: SocketRef| async move {
            warn!(sid = %socket.id, "server: /admin disconnected");
        });
    });

    (layer, io)
}
