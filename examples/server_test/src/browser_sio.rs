#![cfg(feature = "hydrate")]

use std::cell::RefCell;

use js_sys::Function;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    /// socket.io-client 4.x 返回的 Socket 对象。
    #[derive(Clone)]
    pub type JsSocket;

    /// 全局 `io(url)` 函数（socket.io-client 在 window 上暴露）。
    #[wasm_bindgen(js_name = "io")]
    pub fn io_connect(url: &str) -> JsSocket;

    #[wasm_bindgen(method, js_name = "on")]
    pub fn on(this: &JsSocket, event: &str, handler: &Function);

    #[wasm_bindgen(method, js_name = "emit")]
    pub fn emit(this: &JsSocket, event: &str, data: &JsValue);
}

thread_local! {
    static SHARED_SOCKET: RefCell<Option<JsSocket>> = const { RefCell::new(None) };
}

/// 返回浏览器端复用的单例 Socket，避免每次点击按钮都重新建立连接。
pub fn shared_socket() -> JsSocket {
    SHARED_SOCKET.with(|slot| {
        if slot.borrow().is_none() {
            *slot.borrow_mut() = Some(io_connect("/"));
        }
        slot.borrow()
            .as_ref()
            .expect("shared socket should be initialized")
            .clone()
    })
}
