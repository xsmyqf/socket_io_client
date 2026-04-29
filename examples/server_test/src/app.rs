use leptos::prelude::*;
use leptos_meta::{provide_meta_context, MetaTags, Stylesheet, Title};
use leptos_router::{components::{Route, Router, Routes}, StaticSegment};
use serde::{Deserialize, Serialize};

pub fn shell(options: LeptosOptions) -> impl IntoView {
    view! {
        <!DOCTYPE html>
        <html lang="zh-CN">
            <head>
                <meta charset="utf-8"/>
                <meta name="viewport" content="width=device-width, initial-scale=1"/>
                // socket.io-client 必须在 hydrate 脚本之前加载，保证 window.io 可用
                <script src="https://cdn.socket.io/4.7.5/socket.io.min.js"></script>
                <AutoReload options=options.clone() />
                <HydrationScripts options/>
                <MetaTags/>
            </head>
            <body>
                <App/>
            </body>
        </html>
    }
}

#[component]
pub fn App() -> impl IntoView {
    provide_meta_context();
    view! {
        <Stylesheet id="leptos" href="/pkg/server_test.css"/>
        <Title text="socket_io_client demo"/>
        <Router>
            <Routes fallback=|| view! { <p>"404"</p> }>
                <Route path=StaticSegment("") view=Dashboard/>
            </Routes>
        </Router>
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub enum DeviceType {
    #[default]
    WebApp,
    SIOServer,
    SIOClient
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub enum CommandCastMode {
    #[default]
    Broadcast,
    Unicast,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub enum CommandStage {
    #[default]
    Request,
    Response,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CommonCommand {
    pub request_device_id: String,
    pub request_device_type: DeviceType,
    pub request_cast_mode: CommandCastMode,
    pub response_cast_mode: CommandCastMode,
    pub command: String,
    pub params: String,
    pub results: String,
    pub stage: CommandStage,
    pub response_device_id: String,
    pub response_device_type: DeviceType,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RegisterDeviceData {
    pub device_type: DeviceType,
    pub device_id: String,
}

#[component]
fn Dashboard() -> impl IntoView {
    let input1 = RwSignal::new(String::from("field1 value"));
    let input2 = RwSignal::new(String::from("field2 value"));
    let input3 = RwSignal::new(String::from("field3 value"));

    // 日志信号 —— 显示 server 日志
    let (server_logs, set_server_logs) = signal::<Vec<String>>(Vec::new());

    Effect::new(move |_| {
        setup_browser_socket(set_server_logs);
    });

    let on_send_json = {
        let _input1 = input1.clone();
        let _input2 = input2.clone();
        let _input3 = input3.clone();
        move |_| {
            #[cfg(feature = "hydrate")]
            {
                let json_data = serde_json::json!({
                    "field1": _input1.get_untracked(),
                    "field2": _input2.get_untracked(),
                    "field3": _input3.get_untracked(),
                });
                let cmd = CommonCommand {
                    request_device_id: "local_client_1".to_string(),
                    request_device_type: DeviceType::SIOClient,
                    request_cast_mode: CommandCastMode::Unicast,
                    response_cast_mode: CommandCastMode::Unicast,
                    command: "client_to_server_echo_json".to_string(),
                    params: json_data.to_string(),
                    results: "".to_string(),
                    stage: CommandStage::Request,
                    response_device_id: "local_server_1".to_string(),
                    response_device_type: DeviceType::SIOServer,
                };
                if let Ok(json_str) = serde_json::to_string(&cmd) {
                    if let Ok(js_val) = js_sys::JSON::parse(&json_str) {
                        let sock = crate::browser_sio::shared_socket();
                        sock.emit("DealCommonCommand", &js_val);
                    }
                }
            }
        }
    };

    let on_send_multi = {
        let _input1 = input1.clone();
        let _input2 = input2.clone();
        let _input3 = input3.clone();
        move |_| {
            #[cfg(feature = "hydrate")]
            {
                let array_data = serde_json::json!([
                    _input1.get_untracked(),
                    _input2.get_untracked(),
                    _input3.get_untracked(),
                ]);
                let cmd = CommonCommand {
                    request_device_id: "local_client_1".to_string(),
                    request_device_type: DeviceType::SIOClient,
                    request_cast_mode: CommandCastMode::Unicast,
                    response_cast_mode: CommandCastMode::Unicast,
                    command: "client_to_server_echo_multi".to_string(),
                    params: array_data.to_string(),
                    results: "".to_string(),
                    stage: CommandStage::Request,
                    response_device_id: "local_server_1".to_string(),
                    response_device_type: DeviceType::SIOServer,
                };
                if let Ok(json_str) = serde_json::to_string(&cmd) {
                    if let Ok(js_val) = js_sys::JSON::parse(&json_str) {
                        let sock = crate::browser_sio::shared_socket();
                        sock.emit("DealCommonCommand", &js_val);
                    }
                }
            }
        }
    };

    let on_start_tick = {
        move |_| {
            #[cfg(feature = "hydrate")]
            {
                let cmd = CommonCommand {
                    request_device_id: "local_client_1".to_string(),
                    request_device_type: DeviceType::SIOClient,
                    request_cast_mode: CommandCastMode::Unicast,
                    response_cast_mode: CommandCastMode::Unicast,
                    command: "toggle_tick".to_string(),
                    params: "true".to_string(),
                    results: "".to_string(),
                    stage: CommandStage::Request,
                    response_device_id: "local_server_1".to_string(),
                    response_device_type: DeviceType::SIOServer,
                };
                if let Ok(json_str) = serde_json::to_string(&cmd) {
                    if let Ok(js_val) = js_sys::JSON::parse(&json_str) {
                        let sock = crate::browser_sio::shared_socket();
                        sock.emit("DealCommonCommand", &js_val);
                    }
                }
            }
        }
    };

    let on_stop_tick = {
        move |_| {
            #[cfg(feature = "hydrate")]
            {
                let cmd = CommonCommand {
                    request_device_id: "local_client_1".to_string(),
                    request_device_type: DeviceType::SIOClient,
                    request_cast_mode: CommandCastMode::Unicast,
                    response_cast_mode: CommandCastMode::Unicast,
                    command: "toggle_tick".to_string(),
                    params: "false".to_string(),
                    results: "".to_string(),
                    stage: CommandStage::Request,
                    response_device_id: "local_server_1".to_string(),
                    response_device_type: DeviceType::SIOServer,
                };
                if let Ok(json_str) = serde_json::to_string(&cmd) {
                    if let Ok(js_val) = js_sys::JSON::parse(&json_str) {
                        let sock = crate::browser_sio::shared_socket();
                        sock.emit("DealCommonCommand", &js_val);
                    }
                }
            }
        }
    };

    let on_test_tc7 = {
        move |_| {
            #[cfg(feature = "hydrate")]
            {
                let cmd = CommonCommand {
                    request_device_id: "local_client_1".to_string(),
                    request_device_type: DeviceType::SIOClient,
                    request_cast_mode: CommandCastMode::Unicast,
                    response_cast_mode: CommandCastMode::Unicast,
                    command: "trigger_tc7".to_string(),
                    params: "".to_string(),
                    results: "".to_string(),
                    stage: CommandStage::Request,
                    response_device_id: "local_server_1".to_string(),
                    response_device_type: DeviceType::SIOServer,
                };
                if let Ok(json_str) = serde_json::to_string(&cmd) {
                    if let Ok(js_val) = js_sys::JSON::parse(&json_str) {
                        let sock = crate::browser_sio::shared_socket();
                        sock.emit("DealCommonCommand", &js_val);
                    }
                }
            }
        }
    };

    let on_upload_test_zip = {
        move |_| {
            #[cfg(feature = "hydrate")]
            {
                let cmd = CommonCommand {
                    request_device_id: "local_client_1".to_string(),
                    request_device_type: DeviceType::SIOClient,
                    request_cast_mode: CommandCastMode::Unicast,
                    response_cast_mode: CommandCastMode::Unicast,
                    command: "upload_test_zip".to_string(),
                    params: "".to_string(),
                    results: "".to_string(),
                    stage: CommandStage::Request,
                    response_device_id: "local_server_1".to_string(),
                    response_device_type: DeviceType::SIOServer,
                };
                if let Ok(json_str) = serde_json::to_string(&cmd) {
                    if let Ok(js_val) = js_sys::JSON::parse(&json_str) {
                        let sock = crate::browser_sio::shared_socket();
                        sock.emit("DealCommonCommand", &js_val);
                    }
                }
            }
        }
    };

    view! {
        <main class="wrap">
            <h1>"socket_io_client × socketioxide 演示"</h1>
            <p class="page-intro">
                "每个操作卡片都展示了中文说明、参数含义，以及参数在 Rust 代码里如何映射到事件名和 payload。"
            </p>

            <div class="layout">
                <aside class="col-nav">
                    <nav class="feature-nav">
                        <h2>"功能导航"</h2>
                        <a href="#demo-common-command">"功能1：发消息"</a>
                        <a href="#demo-toggle-tick">"功能2：定时发送"</a>
                        <a href="#demo-ack">"功能3：消息确认 (ACKs)"</a>
                        <a href="#demo-binary-upload">"功能4：上传二进制文件"</a>
                    </nav>
                </aside>

                <div class="col-main">
                    <section id="demo-common-command" class="panel anchor-section">
                        <h2>"客户端给服务器端发消息"</h2>
                        <p class="panel-desc">
                            "测试 WebApp 通过 Server 将指令转发给 SIOClient。SIOClient 收到请求后，会通过两种形式给服务器发送数据："
                        </p>
                        <div class="row">
                            <label>"字段 1: "</label>
                            <input
                                type="text"
                                prop:value=move || input1.get()
                                on:input=move |e| input1.set(event_target_value(&e))
                            />
                        </div>
                        <div class="row">
                            <label>"字段 2: "</label>
                            <input
                                type="text"
                                prop:value=move || input2.get()
                                on:input=move |e| input2.set(event_target_value(&e))
                            />
                        </div>
                        <div class="row">
                            <label>"字段 3: "</label>
                            <input
                                type="text"
                                prop:value=move || input3.get()
                                on:input=move |e| input3.set(event_target_value(&e))
                            />
                        </div>
                        <div class="row">
                            <button on:click=on_send_json>"按 JSON 对象发送"</button>
                            <button on:click=on_send_multi>"按多参数发送"</button>
                        </div>
                    </section>

                    <section id="demo-toggle-tick" class="panel anchor-section">
                        <h2>"客户端定时向服务器发送数据"</h2>
                        <p class="panel-desc">
                            "通过发送 toggle_tick 命令，控制 SIOClient 是否每两秒向服务器发送一次 tick 数据。点击按钮可启动或停止。"
                        </p>
                        <div class="row">
                            <button on:click=on_start_tick>"开始发送 tick"</button>
                            <button on:click=on_stop_tick>"停止发送 tick"</button>
                        </div>
                    </section>

                    <section id="demo-ack" class="panel anchor-section">
                        <h2>"功能3：消息确认机制 (ACKs)"</h2>
                        <p class="panel-desc">
                            "测试 Socket.IO 的 ACK 特性。包含客户端向服务端发起带 ACK 的请求（用例 7）。"
                        </p>
                        <div class="row">
                            <button on:click=on_test_tc7>"发送带 ACK 的请求 (客户端->服务端)"</button>
                        </div>
                    </section>

                    <section id="demo-binary-upload" class="panel anchor-section">
                        <h2>"功能4：上传二进制文件"</h2>
                        <p class="panel-desc">
                            "点击按钮后，会通知独立 SIOClient 读取 client_test 目录下的 test.zip，并通过 Socket.IO 二进制附件协议上传到服务器。"
                        </p>
                        <div class="row">
                            <button id="upload-test-zip-btn" on:click=on_upload_test_zip>"上传 client_test/test.zip"</button>
                        </div>
                        <p id="upload-status" class="panel-desc">
                            "按钮会向独立客户端发出上传命令。"
                        </p>
                    </section>
                </div>

                <div class="col-right">
                    <section class="logs logs-full">
                        <h2>"server 日志"</h2>
                        <pre class="log-scroll">
                            {move || server_logs.get()
                                .iter().rev().take(400).cloned()
                                .collect::<Vec<_>>().join("\n")}
                        </pre>
                    </section>
                </div>
            </div>
        </main>
    }
}

#[cfg(feature = "hydrate")]
fn setup_browser_socket(
    set_server_logs: WriteSignal<Vec<String>>,
) {
    use js_sys::Function;
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen::{JsCast, JsValue};

    // 避免 HMR 或 Effect 多次触发时重复连接
    use std::sync::atomic::{AtomicBool, Ordering};
    static CONNECTED: AtomicBool = AtomicBool::new(false);
    if CONNECTED.swap(true, Ordering::SeqCst) {
        return;
    }

    let sock = crate::browser_sio::shared_socket();

    // 注册为 WebApp
    let reg_data = RegisterDeviceData {
        device_type: DeviceType::WebApp,
        device_id: "local_webapp_1".to_string(),
    };
    if let Ok(json_str) = serde_json::to_string(&reg_data) {
        if let Ok(js_val) = js_sys::JSON::parse(&json_str) {
            sock.emit("RegisterDevice", &js_val);
        }
    }

    let push_line = |sig: WriteSignal<Vec<String>>, line: String| {
        let date = js_sys::Date::new_0();
        let time_str = format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
            date.get_full_year(),
            date.get_month() + 1,
            date.get_date(),
            date.get_hours(),
            date.get_minutes(),
            date.get_seconds(),
            date.get_milliseconds()
        );
        let timed_line = format!("[{}] {}", time_str, line);

        sig.update(|v| {
            v.push(timed_line);
            let len = v.len();
            if len > 500 {
                v.drain(..len - 500);
            }
        });
    };

    // log:server
    {
        let cb = Closure::<dyn FnMut(JsValue)>::new(move |data: JsValue| {
            if let Some(line) = data.as_string() {
                push_line(set_server_logs, line);
            }
        });
        let f: &Function = cb.as_ref().unchecked_ref();
        sock.on("log:server", f);
        cb.forget();
    }


}

#[cfg(not(feature = "hydrate"))]
fn setup_browser_socket(
    _set_server_logs: WriteSignal<Vec<String>>,
) {
}
