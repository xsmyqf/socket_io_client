# Socket.IO 客户端集成测试计划

Socket.IO 相比于原生的 WebSocket，最核心的价值就在于它在底层封装了大量复杂的企业级特性。为了保证 Rust 版 Socket.IO 客户端 (`socket_io_client`) 的健壮性，我们需要围绕这些核心特性来构建集成测试（Integration Tests）。

以下是 6 个核心特性维度的测试用例计划，这些测试将存放在 `socket_io_client/tests/` 目录下，通过启动一个本地的测试服务器配合验证。

## 1. 基础连接与生命周期 (Connection & Lifecycle)

Socket.IO 的底层是 Engine.IO，它负责建立连接、升级协议（Polling -> WebSocket）以及心跳保活。

* **测试用例 1：基础连接与断开**
  * **行为**：客户端发起连接，验证是否能触发 `connect` 回调。手动调用 `disconnect`，验证是否触发 `disconnect` 回调。
* **测试用例 2：传输层协议与升级机制 (Transport & Upgrade)**
  * **子用例 2.1：仅使用 WebSocket (WebSocket Only)**
    * **行为**：配置客户端强制使用 `websocket` 传输（禁用 `polling`），验证是否能直连成功并正常收发消息。
  * **子用例 2.2：仅使用 Polling (Polling Only)**
    * **行为**：配置客户端强制使用 `polling` 传输（禁用 `websocket`），验证是否能通过 HTTP 长轮询建立连接并正常收发消息。
  * **子用例 2.3：Polling 升级为 WebSocket (Upgrade)**
    * **行为**：客户端不限制传输方式，默认先通过 `polling` 建立 Engine.IO 连接，验证连接建立后能否成功升级 (Upgrade) 到 `websocket` 协议并继续稳定通信。
* **测试用例 3：心跳维持 (Ping/Pong)**
  * **行为**：建立连接后闲置一段时间（超过服务端的 ping interval），验证连接不会断开，证明客户端底层正确响应了服务端的 Ping 帧。

## 2. 自动重连机制 (Auto-Reconnection)

这是 Socket.IO 最实用的特性之一。当网络波动或服务端重启时，客户端应该能指数退避地尝试重连。

* **测试用例 4：服务端断开后的自动重连**
  * **行为**：建立连接后，强制关闭测试服务器，然后再启动服务器。
  * **验证**：客户端应该依次触发 `disconnect` -> `reconnect_attempt` -> `reconnect` -> `connect` 事件。

## 3. 事件收发 (Event Emission)

不仅仅是发字符串，Socket.IO 支持发送任意 JSON 结构。

* **测试用例 5：基础 Echo 测试 (JSON 数据)**
  * **行为**：客户端向服务端 `emit("echo", {"hello": "world"})`。
  * **验证**：客户端的 `on("echo_reply")` 能准确接收到服务端原样返回的 JSON 数据。
* **测试用例 6：多参数传递 (Multiple Arguments)**
  * **行为**：Socket.IO 允许一个事件带多个参数（打包为数组）。验证客户端能否正确发送和解析包含多个参数的事件。

## 4. 消息确认机制 (Acknowledgements / ACKs)

这是 Socket.IO 独有的类似 RPC 的调用方式，发送消息时带一个回调函数，对方处理完后返回值。

* **测试用例 7：客户端发送带 ACK 的消息**
  * **行为**：客户端使用类似 `emit_with_ack("get_user", 123).await` 发送请求。
  * **验证**：能异步等待并正确获取服务端返回的 `{ "name": "Alice" }` 数据。
* **测试用例 8：客户端响应服务端的 ACK 请求**
  * **行为**：服务端向客户端发送一条带回调的消息，客户端在 `on` 监听器中返回处理结果。
  * **验证**：服务端能正确收到客户端的返回值。

## 5. 命名空间多路复用 (Namespaces Multiplexing)

在一个物理 WebSocket 连接上虚拟出多个独立的通道。

* **测试用例 9：连接并隔离不同的命名空间**
  * **行为**：客户端同时连接 `/` 和 `/admin`。
  * **验证**：向 `/admin` 广播的事件，挂载在 `/` 上的监听器绝对不能收到；反之亦然。

## 6. 二进制数据支持 (Binary Payloads)

Socket.IO 支持直接发送和接收原始的二进制数据（在 Rust 中通常是 `Vec<u8>` 或 `Bytes`），而不需要手动 Base64 编码。

* **测试用例 10：二进制流的发送与接收**
  * **行为**：客户端发送一个包含文本和 `Vec<u8>` 的混合事件（例如文件上传模拟）。
  * **验证**：服务端能正确剥离和解析出二进制流；服务端下发二进制数据时，客户端也能正确解析为字节数组。
