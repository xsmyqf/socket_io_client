# Engine.IO 协议

> 本文翻译自 [Socket.IO 官方文档 - The Engine.IO protocol](https://socket.io/zh-CN/docs/v4/engine-io-protocol/)，介绍 Engine.IO 协议第 4 版。

## 目录

- [简介](#简介)
- [传输方式（Transports）](#传输方式transports)
  - [HTTP 长轮询](#http-长轮询)
    - [请求路径](#请求路径)
    - [查询参数](#查询参数)
    - [请求头](#请求头)
    - [发送与接收数据](#发送与接收数据)
  - [WebSocket](#websocket)
- [协议（Protocol）](#协议protocol)
  - [握手（Handshake）](#握手handshake)
  - [心跳（Heartbeat）](#心跳heartbeat)
  - [升级（Upgrade）](#升级upgrade)
  - [消息（Message）](#消息message)
- [数据包编码](#数据包编码)
  - [HTTP 长轮询](#http-长轮询-1)
  - [WebSocket](#websocket-1)
- [版本历史](#版本历史)
  - [从 v2 到 v3](#从-v2-到-v3)
  - [从 v3 到 v4](#从-v3-到-v4)
- [测试套件](#测试套件)

---

## 简介

> Engine.IO 协议支持客户端与服务器之间的**全双工**、**低开销**通信。

该协议基于 WebSocket 协议，并在无法建立 WebSocket 连接时，回退到 HTTP 长轮询作为备用机制。

官方参考实现使用 TypeScript 编写：

- 服务端：<https://github.com/socketio/engine.io>
- 客户端：<https://github.com/socketio/engine.io-client>

Socket.IO 协议在 Engine.IO 的基础上构建，为通信通道补充了额外的特性。

---

## 传输方式（Transports）

Engine.IO 客户端与服务器之间的连接可以通过以下两种方式建立：

- HTTP 长轮询
- WebSocket

### HTTP 长轮询

HTTP 长轮询传输由一系列连续的 HTTP 请求组成：

- 长连接 GET 请求：用于接收服务器推送的数据；
- 短连接 POST 请求：用于客户端向服务端发送数据。

#### 请求路径

默认的 HTTP 请求路径为 `/engine.io/`。基于该协议构建的库可以修改此路径，例如 Socket.IO 使用 `/socket.io/`。

#### 查询参数

| 名称 | 取值 | 说明 |
|------|------|------|
| `EIO` | `4` | 必填，协议版本 |
| `transport` | `polling` | 必填，传输方式名称 |
| `sid` | `<sid>` | 会话建立之后必填，会话标识符 |

若缺少必填查询参数，服务器必须返回 HTTP 400 错误。

#### 请求头

传输二进制数据时，发送方必须添加请求头 `Content-Type: application/octet-stream`。若未显式设置 content-type，接收方应将数据视为纯文本处理。

#### 发送与接收数据

##### 发送数据

客户端通过 HTTP POST 请求将编码后的数据包放在请求体中发送：

```
CLIENT                                                 SERVER

  │                                                      │
  │   POST /engine.io/?EIO=4&transport=polling&sid=...   │
  │ ───────────────────────────────────────────────────► │
  │ ◄──────────────────────────────────────────────────┘ │
  │                        HTTP 200                      │
  │                                                      │
```

若会话 ID 未知，服务器必须返回 HTTP 400。请求成功时返回 HTTP 200，响应体为 `ok`。

> 为保证数据包顺序，客户端同一时刻**不得**有一个以上处于活动状态的 POST 请求。

##### 接收数据

客户端通过 HTTP GET 请求接收数据包：

```
CLIENT                                                SERVER

  │   GET /engine.io/?EIO=4&transport=polling&sid=...   │
  │ ──────────────────────────────────────────────────► │
  │                                                   . │
  │                                                   . │
  │                                                   . │
  │                                                   . │
  │ ◄─────────────────────────────────────────────────┘ │
  │                       HTTP 200                      │
```

若会话 ID 未知，服务器必须返回 HTTP 400。若当前无缓冲的数据包，服务器可延迟响应；一旦有数据可发送，应将数据包编码后放入响应体返回。

> 为保证数据包顺序，客户端同一时刻**不得**有一个以上处于活动状态的 GET 请求。

### WebSocket

WebSocket 传输在服务端与客户端之间提供双向、低延迟的通信能力。

使用的查询参数：

| 名称 | 取值 | 说明 |
|------|------|------|
| `EIO` | `4` | 必填，协议版本 |
| `transport` | `websocket` | 必填，传输方式名称 |
| `sid` | `<sid>` | 可选，取决于是否由 HTTP 长轮询升级而来 |

若缺少必填参数，服务器必须关闭该连接。

> 每个数据包（无论是读还是写）都在各自独立的 WebSocket 帧中发送。

> 每个会话客户端**不得**建立一个以上的 WebSocket 连接。

---

## 协议（Protocol）

一个 Engine.IO 数据包由下列部分构成：

- 数据包类型（packet type）
- 可选的数据包负载（packet payload）

可用的数据包类型：

| 类型 | ID | 用途 |
|------|----|----|
| open | 0 | 握手阶段使用 |
| close | 1 | 表示传输通道可以关闭 |
| ping | 2 | 心跳机制中使用 |
| pong | 3 | 心跳机制中使用 |
| message | 4 | 向对端发送负载数据 |
| upgrade | 5 | 传输升级过程中使用 |
| noop | 6 | 传输升级过程中使用 |

### 握手（Handshake）

客户端通过 HTTP GET 请求向服务器发起连接建立。

先使用 HTTP 长轮询（默认方式）：

```
CLIENT                                                    SERVER

  │                                                          │
  │        GET /engine.io/?EIO=4&transport=polling           │
  │ ───────────────────────────────────────────────────────► │
  │ ◄──────────────────────────────────────────────────────┘ │
  │                        HTTP 200                          │
  │                                                          │
```

仅使用 WebSocket 的会话：

```
CLIENT                                                    SERVER

  │                                                          │
  │        GET /engine.io/?EIO=4&transport=websocket         │
  │ ───────────────────────────────────────────────────────► │
  │ ◄──────────────────────────────────────────────────────┘ │
  │                        HTTP 101                          │
  │                                                          │
```

当服务器接受连接时，会返回一个 `open` 数据包，其 JSON 负载包含以下字段：

| 键名 | 类型 | 说明 |
|------|------|------|
| `sid` | `string` | 会话 ID |
| `upgrades` | `string[]` | 可用的传输升级选项 |
| `pingInterval` | `number` | 心跳 ping 的间隔（毫秒） |
| `pingTimeout` | `number` | 心跳 ping 的超时时间（毫秒） |
| `maxPayload` | `number` | 客户端聚合数据包时每段的最大字节数 |

示例：

```json
{
  "sid": "lv_VI97HAXpY6yYWAAAC",
  "upgrades": ["websocket"],
  "pingInterval": 25000,
  "pingTimeout": 20000,
  "maxPayload": 1000000
}
```

> 客户端在之后所有的请求中，**必须**在查询参数中携带 `sid` 值。

### 心跳（Heartbeat）

握手完成之后，通过心跳机制来验证连接是否仍然存活：

```
CLIENT                                                 SERVER

  │                   *** Handshake ***                  │
  │                                                      │
  │  ◄─────────────────────────────────────────────────  │
  │                           2                          │  (ping packet)
  │  ─────────────────────────────────────────────────►  │
  │                           3                          │  (pong packet)
```

> 按照固定的时间间隔（握手时返回的 `pingInterval` 值），服务器发送一个 ping 包，客户端必须在若干秒内（`pingTimeout` 值）回复一个 pong 包。

若服务器未收到 pong 响应，应认为连接已关闭；若客户端在 `pingInterval + pingTimeout` 时间内未收到 ping 数据包，也应认为连接已关闭。

### 升级（Upgrade）

> 默认情况下，客户端**应当**先建立 HTTP 长轮询连接，之后在条件允许时升级到更优的传输方式。

要升级至 WebSocket，客户端必须：

- 暂停 HTTP 长轮询传输，以避免数据包丢失；
- 使用相同的会话 ID 建立 WebSocket 连接；
- 发送一个 ping 数据包，负载为字符串 `probe`。

服务器必须：

- 向挂起的 GET 请求发送一个 noop 数据包，以便干净地关闭 HTTP 长轮询；
- 回复一个 pong 数据包，负载为字符串 `probe`。

随后客户端必须发送 upgrade 数据包完成升级：

```
CLIENT                                                 SERVER

  │                                                      │
  │   GET /engine.io/?EIO=4&transport=websocket&sid=...  │
  │ ───────────────────────────────────────────────────► │
  │  ◄─────────────────────────────────────────────────┘ │
  │            HTTP 101 (WebSocket handshake)            │
  │                                                      │
  │            -----  WebSocket frames -----             │
  │  ─────────────────────────────────────────────────►  │
  │                         2probe                       │ (ping 数据包)
  │  ◄─────────────────────────────────────────────────  │
  │                         3probe                       │ (pong 数据包)
  │  ─────────────────────────────────────────────────►  │
  │                         5                            │ (upgrade 数据包)
  │                                                      │
```

### 消息（Message）

握手完成之后，客户端与服务器可通过 message 数据包相互交换数据。

---

## 数据包编码

Engine.IO 数据包的序列化方式取决于两点：负载类型（纯文本或二进制）、传输方式。字符编码使用 UTF-8；二进制负载采用 base64 编码。

### HTTP 长轮询

为提升吞吐量，可以将多个数据包拼接在一起发送。

格式：

```
<数据包类型>[<数据>]<分隔符><数据包类型>[<数据>]<分隔符><数据包类型>[<数据>][...]
```

示例：

```
4hello\x1e2\x1e4world

其中：

4      => message 数据包类型
hello  => message 负载
\x1e   => 分隔符
2      => ping 数据包类型
\x1e   => 分隔符
4      => message 数据包类型
world  => message 负载
```

数据包之间使用记录分隔符 `\x1e` 隔开。

> 二进制负载**必须**先以 base64 编码，并在前面添加前缀字符 `b`：

示例：

```
4hello\x1ebAQIDBA==

其中：

4         => message 数据包类型
hello     => message 负载
\x1e      => 分隔符
b         => 二进制前缀
AQIDBA==  => 缓冲区 <01 02 03 04> 的 base64 编码
```

> 客户端**应当**依据握手阶段获得的 `maxPayload` 值，来决定一次拼接多少个数据包。

### WebSocket

> 每个 Engine.IO 数据包都在独立的 WebSocket 帧中发送。

格式：

```
<数据包类型>[<数据>]
```

示例：

```
4hello

其中：

4      => message 数据包类型
hello  => message 负载（UTF-8 编码）
```

二进制负载无需任何转换，直接发送。

---

## 版本历史

### 从 v2 到 v3

- 新增对二进制数据的支持。

版本 2 用于 Socket.IO v0.9 及更早版本。

版本 3 用于 Socket.IO v1 与 v2。

### 从 v3 到 v4

- 反转 ping/pong 机制

> 现在由服务器来发送 ping 数据包，因为浏览器中设置的定时器不够可靠。

- 对包含二进制数据的负载，始终使用 base64 编码

这样就不必关心客户端或传输层是否支持二进制数据，所有负载都能被一致处理。该规则仅适用于 HTTP 长轮询；WebSocket 的二进制数据仍无需做任何转换。

- 使用记录分隔符（`\x1e`）替代字符计数

> 使用字符计数会阻碍（或至少使之更困难）用其他语言实现该协议，因为这些语言未必使用 UTF-16 编码。

例如：`€` 原本被编码为 `2:4€`，但 `Buffer.byteLength('€') === 3`。

注意：此方案假定数据中不会出现记录分隔符字符。

版本 4（当前版本）被 Socket.IO v3 及之后的版本所采用。

---

## 测试套件

位于 `test-suite/` 目录下的测试套件可用于检查服务端实现的合规性。

使用方法：

- 在 Node.js 中运行：`npm ci && npm test`
- 在浏览器中运行：直接打开目录下的 `index.html` 文件

一份可通过全部测试的 JavaScript 服务端参考配置：

```javascript
import { listen } from "engine.io";

const server = listen(3000, {
  pingInterval: 300,
  pingTimeout: 200,
  maxPayload: 1e6,
  cors: {
    origin: "*"
  }
});

server.on("connection", socket => {
  socket.on("data", (...args) => {
    socket.send(...args);
  });
});
```
