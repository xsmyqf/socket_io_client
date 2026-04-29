# Socket.IO 协议

> 本文翻译自 [Socket.IO 官方文档 - The Socket.IO protocol](https://socket.io/zh-CN/docs/v4/socket-io-protocol/)，描述 Socket.IO 协议第 5 版。

## 目录

- [简介](#简介)
- [交换协议（Exchange protocol）](#交换协议exchange-protocol)
  - [连接到命名空间](#连接到命名空间)
  - [发送与接收数据](#发送与接收数据)
  - [确认（Acknowledgement）](#确认acknowledgement)
  - [从命名空间断开](#从命名空间断开)
- [数据包编码](#数据包编码)
  - [格式](#格式)
  - [示例](#示例)
    - [连接到命名空间](#连接到命名空间-1)
    - [发送与接收数据](#发送与接收数据-1)
    - [确认](#确认)
    - [从命名空间断开](#从命名空间断开-1)
- [会话样例](#会话样例)
- [版本历史](#版本历史)
  - [v5 与 v4 的差异](#v5-与-v4-的差异)
  - [v4 与 v3 的差异](#v4-与-v3-的差异)
  - [v3 与 v2 的差异](#v3-与-v2-的差异)
  - [v2 与 v1 的差异](#v2-与-v1-的差异)
  - [初始版本](#初始版本)
- [测试套件](#测试套件)

---

## 简介

Socket.IO 协议支持客户端与服务器之间的**全双工**、**低开销**通信。

它构建在 Engine.IO 协议之上，由后者负责处理 WebSocket 与 HTTP 长轮询相关的底层细节。

Socket.IO 协议在此基础上提供以下额外特性：

- **多路复用**（在 Socket.IO 中称为“命名空间”，namespace）

JavaScript API 示例：

_服务端_

```js
const namespace = io.of("/admin");
namespace.on("connection", (socket) => {
  // ...
});
```

_客户端_

```js
const socket1 = io();
const socket2 = io("/admin");
socket2.on("connect", () => {
  // ...
});
```

- **数据包确认**

JavaScript API 示例：

```js
socket.emit("hello", "foo", (arg) => {
  console.log("received", arg);
});

socket.on("hello", (arg, ack) => {
  ack("bar");
});
```

官方参考实现使用 TypeScript 编写：

- 服务端：<https://github.com/socketio/socket.io>
- 客户端：<https://github.com/socketio/socket.io-client>

---

## 交换协议（Exchange protocol）

一个 Socket.IO 数据包包含以下字段：

- 数据包类型（整数）
- 命名空间（字符串）
- 可选的负载（Object | Array）
- 可选的确认 ID（整数）

可用的数据包类型列表：

| 类型 | ID | 用途 |
|------|----|----|
| CONNECT | 0 | 用于连接到某个命名空间 |
| DISCONNECT | 1 | 用于断开某个命名空间 |
| EVENT | 2 | 用于向对端发送数据 |
| ACK | 3 | 用于确认一个事件 |
| CONNECT_ERROR | 4 | 用于连接到命名空间失败时 |
| BINARY_EVENT | 5 | 用于向对端发送二进制数据 |
| BINARY_ACK | 6 | 用于确认一个事件（响应包含二进制数据） |

### 连接到命名空间

在一次 Socket.IO 会话开始时，客户端**必须**发送一个 `CONNECT` 数据包。

服务端**必须**回复以下两者之一：

- 若连接成功，返回一个 `CONNECT` 数据包，负载中包含会话 ID；
- 若连接被拒绝，返回一个 `CONNECT_ERROR` 数据包。

```
CLIENT                                                      SERVER

  │  ───────────────────────────────────────────────────────►  │
  │             { type: CONNECT, namespace: "/" }              │
  │  ◄───────────────────────────────────────────────────────  │
  │   { type: CONNECT, namespace: "/", data: { sid: "..." } }  │
```

若服务端首先收到的不是 `CONNECT` 数据包，则**必须**立即关闭连接。

一个客户端**可以**通过同一条底层 WebSocket 连接，同时连接到多个命名空间。

示例：

- 连接主命名空间（名称为 `"/"`）

```
Client > { type: CONNECT, namespace: "/" }
Server > { type: CONNECT, namespace: "/", data: { sid: "wZX3oN0bSVIhsaknAAAI" } }
```

- 连接自定义命名空间

```
Client > { type: CONNECT, namespace: "/admin" }
Server > { type: CONNECT, namespace: "/admin", data: { sid: "oSO0OpakMV_3jnilAAAA" } }
```

- 附带额外负载

```
Client > { type: CONNECT, namespace: "/admin", data: { "token": "123" } }
Server > { type: CONNECT, namespace: "/admin", data: { sid: "iLnRaVGHY4B75TeVAAAB" } }
```

- 连接被拒绝

```
Client > { type: CONNECT, namespace: "/" }
Server > { type: CONNECT_ERROR, namespace: "/", data: { message: "Not authorized" } }
```

### 发送与接收数据

在连接到某个命名空间之后，客户端与服务端可以开始交换数据：

```
CLIENT                                                      SERVER

  │  ───────────────────────────────────────────────────────►  │
  │        { type: EVENT, namespace: "/", data: ["foo"] }      │
  │                                                            │
  │  ◄───────────────────────────────────────────────────────  │
  │        { type: EVENT, namespace: "/", data: ["bar"] }      │
```

负载是必填项，**必须**为非空数组。否则接收方**必须**关闭连接。

示例：

- 主命名空间

```
Client > { type: EVENT, namespace: "/", data: ["foo"] }
```

- 自定义命名空间

```
Server > { type: EVENT, namespace: "/admin", data: ["bar"] }
```

- 携带二进制数据

```
Client > { type: BINARY_EVENT, namespace: "/", data: ["baz", <Buffer <01 02 03 04>> ] }
```

### 确认（Acknowledgement）

发送方**可以**附带一个事件 ID，以便向接收方请求确认：

```
CLIENT                                                      SERVER

  │  ───────────────────────────────────────────────────────►  │
  │   { type: EVENT, namespace: "/", data: ["foo"], id: 12 }   │
  │  ◄───────────────────────────────────────────────────────  │
  │    { type: ACK, namespace: "/", data: ["bar"], id: 12 }    │
```

接收方**必须**返回一个 `ACK` 数据包，并使用相同的事件 ID。

负载是必填项，**必须**为一个数组（可以为空数组）。

示例：

- 主命名空间

```
Client > { type: EVENT, namespace: "/", data: ["foo"], id: 12 }
Server > { type: ACK, namespace: "/", data: [], id: 12 }
```

- 自定义命名空间

```
Server > { type: EVENT, namespace: "/admin", data: ["foo"], id: 13 }
Client > { type: ACK, namespace: "/admin", data: ["bar"], id: 13 }
```

- 携带二进制数据

```
Client > { type: BINARY_EVENT, namespace: "/", data: ["foo", <buffer <01 02 03 04> ], id: 14 }
Server > { type: ACK, namespace: "/", data: ["bar"], id: 14 }

或：

Server > { type: EVENT, namespace: "/", data: ["foo" ], id: 15 }
Client > { type: BINARY_ACK, namespace: "/", data: ["bar", <buffer <01 02 03 04>], id: 15 }
```

### 从命名空间断开

任意一方都可以在任意时刻发送 `DISCONNECT` 数据包，以结束与某命名空间的连接：

```
CLIENT                                                      SERVER

  │  ───────────────────────────────────────────────────────►  │
  │           { type: DISCONNECT, namespace: "/" }             │
```

对端无需响应该包。若客户端仍连接着其他命名空间，底层连接**可以**继续保持存活。

---

## 数据包编码

本节描述 Socket.IO 默认解析器所使用的编码方式，该解析器内置于 Socket.IO 服务端与客户端中，源码位于 <https://github.com/socketio/socket.io-parser>。

JavaScript 版本的服务端与客户端也支持自定义解析器，不同解析器各有取舍，针对不同场景可能更合适，例如 `socket.io-json-parser` 或 `socket.io-msgpack-parser`。

另外请注意，每一个 Socket.IO 数据包都会被放在 Engine.IO 的 `message` 数据包中发送（详见 <https://github.com/socketio/engine.io-protocol>），因此真正发出去时（HTTP 长轮询的请求/响应体中，或 WebSocket 帧中），编码结果前会再加上字符 `"4"`。

### 格式

```
<数据包类型>[<二进制附件数量>-][<命名空间>,][<确认 ID>][不含二进制的 JSON 序列化负载]

+ 附加的二进制数据
```

注意：只有当命名空间与主命名空间（`/`）不同，才会在编码中包含命名空间部分。

### 示例

#### 连接到命名空间

- 主命名空间

_数据包_

```
{ type: CONNECT, namespace: "/" }
```

_编码后_

```
0
```

- 自定义命名空间

_数据包_

```
{ type: CONNECT, namespace: "/admin", data: { sid: "oSO0OpakMV_3jnilAAAA" } }
```

_编码后_

```
0/admin,{"sid":"oSO0OpakMV_3jnilAAAA"}
```

- 连接被拒绝

_数据包_

```
{ type: CONNECT_ERROR, namespace: "/", data: { message: "Not authorized" } }
```

_编码后_

```
4{"message":"Not authorized"}
```

#### 发送与接收数据

- 主命名空间

_数据包_

```
{ type: EVENT, namespace: "/", data: ["foo"] }
```

_编码后_

```
2["foo"]
```

- 自定义命名空间

_数据包_

```
{ type: EVENT, namespace: "/admin", data: ["bar"] }
```

_编码后_

```
2/admin,["bar"]
```

- 携带二进制数据

_数据包_

```
{ type: BINARY_EVENT, namespace: "/", data: ["baz", <Buffer <01 02 03 04>> ] }
```

_编码后_

```
51-["baz",{"_placeholder":true,"num":0}]

+ <Buffer <01 02 03 04>>
```

- 多个二进制附件

_数据包_

```
{ type: BINARY_EVENT, namespace: "/admin", data: ["baz", <Buffer <01 02>>, <Buffer <03 04>> ] }
```

_编码后_

```
52-/admin,["baz",{"_placeholder":true,"num":0},{"_placeholder":true,"num":1}]

+ <Buffer <01 02>>
+ <Buffer <03 04>>
```

再次提醒：每个 Socket.IO 数据包都会被包装在 Engine.IO 的 `message` 数据包中，所以真正上线时会再加上前缀字符 `"4"`。

例如：`{ type: EVENT, namespace: "/", data: ["foo"] }` 实际会以 `42["foo"]` 形式发送。

#### 确认

- 主命名空间

_数据包_

```
{ type: EVENT, namespace: "/", data: ["foo"], id: 12 }
```

_编码后_

```
212["foo"]
```

- 自定义命名空间

_数据包_

```
{ type: ACK, namespace: "/admin", data: ["bar"], id: 13 }
```

_编码后_

```
3/admin,13["bar"]
```

- 携带二进制数据

_数据包_

```
{ type: BINARY_ACK, namespace: "/", data: ["bar", <Buffer <01 02 03 04>>], id: 15 }
```

_编码后_

```
61-15["bar",{"_placeholder":true,"num":0}]

+ <Buffer <01 02 03 04>>
```

#### 从命名空间断开

- 主命名空间

_数据包_

```
{ type: DISCONNECT, namespace: "/" }
```

_编码后_

```
1
```

- 自定义命名空间

_数据包_

```
{ type: DISCONNECT, namespace: "/admin" }
```

_编码后_

```
1/admin,
```

---

## 会话样例

下例展示将 Engine.IO 协议与 Socket.IO 协议结合起来时，实际在网络上传输的数据：

- 第 1 次请求（open 数据包）

```
GET /socket.io/?EIO=4&transport=polling&t=N8hyd6w
< HTTP/1.1 200 OK
< Content-Type: text/plain; charset=UTF-8
0{"sid":"lv_VI97HAXpY6yYWAAAC","upgrades":["websocket"],"pingInterval":25000,"pingTimeout":5000,"maxPayload":1000000}
```

详情：

```
0           => Engine.IO "open" 数据包类型
{"sid":...  => Engine.IO 握手数据
```

注意：查询参数 `t` 用于防止浏览器缓存该请求。

- 第 2 次请求（命名空间连接请求）

```
POST /socket.io/?EIO=4&transport=polling&t=N8hyd7H&sid=lv_VI97HAXpY6yYWAAAC
< HTTP/1.1 200 OK
< Content-Type: text/plain; charset=UTF-8
40
```

详情：

```
4           => Engine.IO "message" 数据包类型
0           => Socket.IO "CONNECT" 数据包类型
```

- 第 3 次请求（命名空间连接批准）

```
GET /socket.io/?EIO=4&transport=polling&t=N8hyd7H&sid=lv_VI97HAXpY6yYWAAAC
< HTTP/1.1 200 OK
< Content-Type: text/plain; charset=UTF-8
40{"sid":"wZX3oN0bSVIhsaknAAAI"}
```

- 第 4 次请求

服务端执行 `socket.emit('hey', 'Jude')`：

```
GET /socket.io/?EIO=4&transport=polling&t=N8hyd7H&sid=lv_VI97HAXpY6yYWAAAC
< HTTP/1.1 200 OK
< Content-Type: text/plain; charset=UTF-8
42["hey","Jude"]
```

详情：

```
4           => Engine.IO "message" 数据包类型
2           => Socket.IO "EVENT" 数据包类型
[...]       => 内容
```

- 第 5 次请求（消息外发）

客户端执行 `socket.emit('hello'); socket.emit('world');`：

```
POST /socket.io/?EIO=4&transport=polling&t=N8hzxke&sid=lv_VI97HAXpY6yYWAAAC
> Content-Type: text/plain; charset=UTF-8
42["hello"]\x1e42["world"]
< HTTP/1.1 200 OK
< Content-Type: text/plain; charset=UTF-8
ok
```

详情：

```
4           => Engine.IO "message" 数据包类型
2           => Socket.IO "EVENT" 数据包类型
["hello"]   => 第 1 个内容
\x1e        => 分隔符
4           => Engine.IO "message" 数据包类型
2           => Socket.IO "EVENT" 数据包类型
["world"]   => 第 2 个内容
```

- 第 6 次请求（升级到 WebSocket）

```
GET /socket.io/?EIO=4&transport=websocket&sid=lv_VI97HAXpY6yYWAAAC
< HTTP/1.1 101 Switching Protocols
```

WebSocket 帧序列：

```
< 2probe                                        => Engine.IO probe 请求
> 3probe                                        => Engine.IO probe 响应
> 5                                             => Engine.IO "upgrade" 数据包类型
> 42["hello"]
> 42["world"]
> 40/admin,                                     => 请求接入 admin 命名空间（Socket.IO "CONNECT" 数据包）
< 40/admin,{"sid":"-G5j-67EZFp-q59rADQM"}       => 批准接入 admin 命名空间
> 42/admin,1["tellme"]                          => 带确认的 Socket.IO "EVENT" 数据包
< 461-/admin,1[{"_placeholder":true,"num":0}]   => 带占位符的 Socket.IO "BINARY_ACK" 数据包
< <binary>                                      => 对应的二进制附件（紧接着下一帧发送）
... 一段时间没有消息
> 2                                             => Engine.IO "ping" 数据包类型
< 3                                             => Engine.IO "pong" 数据包类型
> 1                                             => Engine.IO "close" 数据包类型
```

---

## 版本历史

### v5 与 v4 的差异

Socket.IO 协议的第 5 版（当前版本）被 Socket.IO v3 及更高版本使用（`v3.0.0` 于 2020 年 11 月发布）。

它构建在 Engine.IO 协议第 4 版之上（这也是查询参数 `EIO=4` 的由来）。

变更列表：

- **移除了对默认命名空间的隐式连接**

在旧版本中，即使客户端请求访问另一个命名空间，它仍然会被隐式地连接到默认命名空间。

现在不再如此，客户端在任何情况下都必须显式发送一个 `CONNECT` 数据包。

提交：`09b6f23`（服务端）与 `249e0be`（客户端）

- **将 `ERROR` 重命名为 `CONNECT_ERROR`**

其含义与代码编号（4）均未变化，该类型仍用于服务端拒绝命名空间连接的场景，只是新名称更能体现其用途。

提交：`d16c035`（服务端）与 `13e1db7c`（客户端）

- **`CONNECT` 数据包现在可以携带负载**

客户端可发送一段负载以用于认证/鉴权。示例：

```
{
  "type": 0,
  "nsp": "/admin",
  "data": {
    "token": "123"
  }
}
```

连接成功时，服务端会响应包含该 Socket ID 的负载。示例：

```
{
  "type": 0,
  "nsp": "/admin",
  "data": {
    "sid": "CjdVH4TQvovi1VvgAC5Z"
  }
}
```

这一变化意味着 Socket.IO 连接的 ID，将不再等于底层 Engine.IO 连接的 ID（后者可在 HTTP 请求的查询参数中看到）。

提交：`2875d2c`（服务端）与 `bbe94ad`（客户端）

- **`CONNECT_ERROR` 数据包的负载由原先的纯字符串变为对象**

提交：`54bf4a4`（服务端）与 `0939395`（客户端）

### v4 与 v3 的差异

Socket.IO 协议第 4 版被 Socket.IO v1（`v1.0.3` 于 2014 年 6 月发布）与 v2（`v2.0.0` 于 2017 年 5 月发布）使用。

详情见：<https://github.com/socketio/socket.io-protocol/tree/v4>

它构建在 Engine.IO 协议第 3 版之上（对应查询参数 `EIO=3`）。

变更列表：

- **新增 `BINARY_ACK` 数据包类型**

在此之前，`ACK` 数据包总是被视为可能包含二进制对象，因此需要递归查找其中的二进制对象，这会影响性能。

参考：<https://github.com/socketio/socket.io-parser/commit/ca4f42a922ba7078e840b1bc09fe3ad618acc065>

### v3 与 v2 的差异

Socket.IO 协议第 3 版被早期的 Socket.IO v1 版本使用（`socket.io@1.0.0...1.0.2`，发布于 2014 年 5 月）。

详情见：<https://github.com/socketio/socket.io-protocol/tree/v3>

变更列表：

- 对含二进制对象的数据包不再使用 msgpack 编码（另见 `299849b`）。

### v2 与 v1 的差异

变更列表：

- **新增 `BINARY_EVENT` 数据包类型**

此变更是在 Socket.IO 1.0 的开发过程中引入的，目的是支持二进制对象。`BINARY_EVENT` 数据包最初使用 msgpack 编码。

### 初始版本

最早的版本是在将 Engine.IO 协议（负责 WebSocket/HTTP 长轮询等底层细节与心跳）与 Socket.IO 协议拆分之后产生的。它从未随任何一版 Socket.IO 发布，但为后续的迭代奠定了基础。

---

## 测试套件

位于 `test-suite/` 目录下的测试套件可用于检验服务端实现是否符合协议。

使用方法：

- 在 Node.js 中运行：`npm ci && npm test`
- 在浏览器中运行：直接打开 `index.html` 文件

一份可通过全部测试的 JavaScript 服务端参考配置：

```javascript
import { Server } from "socket.io";

const io = new Server(3000, {
  pingInterval: 300,
  pingTimeout: 200,
  maxPayload: 1000000,
  connectTimeout: 1000,
  cors: {
    origin: "*"
  }
});

io.on("connection", (socket) => {
  socket.emit("auth", socket.handshake.auth);

  socket.on("message", (...args) => {
    socket.emit.apply(socket, ["message-back", ...args]);
  });

  socket.on("message-with-ack", (...args) => {
    const ack = args.pop();
    ack(...args);
  })
});

io.of("/custom").on("connection", (socket) => {
  socket.emit("auth", socket.handshake.auth);
});
```

---

## 许可证

MIT
