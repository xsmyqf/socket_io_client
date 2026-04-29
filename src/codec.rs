//! Socket.IO 协议的报文编解码辅助。
//! Socket.IO protocol packet encoding and decoding helpers.
//!
//! 协议格式（参见 `doc/socket-io-protocol.md`）：
//! Protocol format (see `doc/socket-io-protocol.md`):
//!
//! ```text
//! <packet type>[<# of binary attachments>-][<namespace>,][<ack id>]<JSON payload>
//! ```

/// 解析 Socket.IO 报文的 Namespace 和剩余载荷。
/// Parses the Namespace and remaining payload of a Socket.IO packet.
///
/// 按照协议规范，如果载荷以 `/` 开头，则认为指明了 Namespace，并且后面必须跟着 `,` 和具体数据。
/// According to the protocol specification, if the payload starts with `/`, it is considered to specify a Namespace, and must be followed by `,` and the specific data.
///
/// # 示例 (Examples)
/// - `/chat,["event",{...}]` -> `("/chat", "[\"event\",{...}]")`
/// - `["event",{...}]`       -> `("/",    "[\"event\",{...}]")`
///
/// # 返回值 (Returns)
/// 返回一个元组 `(Namespace 字符串, 剩余的载荷字符串)`。
/// Returns a tuple `(Namespace string, remaining payload string)`.
pub fn parse_ns_payload(s: &str) -> (String, &str) {
    if s.is_empty() {
        return ("/".to_string(), s);
    }

    if s.starts_with('/') {
        if let Some(idx) = s.find(',') {
            let ns = &s[..idx];
            let rest = &s[idx + 1..];
            (ns.to_string(), rest)
        } else {
            (s.to_string(), "")
        }
    } else {
        ("/".to_string(), s)
    }
}

/// 在解析 Namespace 基础上，继续提取报文中的 Ack ID。
/// On the basis of parsing the Namespace, continues to extract the Ack ID from the packet.
///
/// Socket.IO 允许在 Namespace 后面紧跟一段数字作为 ACK ID（用于实现请求-响应模式），
/// Socket.IO allows a string of numbers to immediately follow the Namespace as an ACK ID (used to implement the request-response pattern),
/// 之后才是真正的 JSON 载荷。
/// followed by the actual JSON payload.
///
/// # 返回值 (Returns)
/// 返回一个元组 `(Namespace 字符串, 提取到的 Ack ID（可选）, 剩余的载荷字符串)`。
/// Returns a tuple `(Namespace string, extracted Ack ID (optional), remaining payload string)`.
pub fn parse_ns_payload_with_id(s: &str) -> (String, Option<u64>, &str) {
    let (ns, rest) = parse_ns_payload(s);

    let bytes = rest.as_bytes();
    if !bytes.is_empty() && bytes[0].is_ascii_digit() {
        let mut end = 0;
        while end < bytes.len() && bytes[end].is_ascii_digit() {
            end += 1;
        }
        let id_str = &rest[..end];
        let id = id_str.parse::<u64>().ok();
        let payload = &rest[end..];
        (ns, id, payload)
    } else {
        (ns, None, rest)
    }
}

/// 解析 BINARY_EVENT / BINARY_ACK 报文的附件数量前缀 `<N>-`。
/// Parses the attachment count prefix `<N>-` of a BINARY_EVENT / BINARY_ACK packet.
///
/// 在发送带有二进制附件的消息时，载荷最前面会附加附件的数量 `N` 和一个破折号 `-`。
/// When sending a message with binary attachments, the payload is prefixed with the number of attachments `N` and a dash `-`.
///
/// # 示例 (Examples)
/// - `1-/ns,["evt",{...}]` -> `(1, "/ns,[\"evt\",{...}]")`
/// - `2-["evt",{...},{...}]` -> `(2, "[\"evt\",{...},{...}]")`
///
/// # 返回值 (Returns)
/// 返回一个元组 `(附件数量, 剩余的载荷字符串)`。如果解析不到，返回 `(0, 原始字符串)`。
/// Returns a tuple `(number of attachments, remaining payload string)`. If it cannot be parsed, returns `(0, original string)`.
pub fn parse_binary_header(s: &str) -> (usize, &str) {
    let bytes = s.as_bytes();
    let mut end = 0;
    while end < bytes.len() && bytes[end].is_ascii_digit() {
        end += 1;
    }
    if end > 0 && end < bytes.len() && bytes[end] == b'-' {
        let n = s[..end].parse::<usize>().unwrap_or(0);
        (n, &s[end + 1..])
    } else {
        (0, s)
    }
}

/// 编码并生成普通的文本事件报文字符串。
/// Encodes and generates a normal text event packet string.
///
/// 格式为 `<类型>[<Namespace>,]<载荷>`。
/// Format is `<type>[<Namespace>,]<payload>`.
///
/// # 参数 (Parameters)
/// * `ns`: Namespace（例如 `/chat` 或 `/`）
/// * `ns`: Namespace (e.g., `/chat` or `/`)
/// * `typ`: 报文类型标识（例如 `"42"` 表示 Socket.IO 事件报文）
/// * `typ`: Packet type identifier (e.g., `"42"` for a Socket.IO event packet)
/// * `payload`: 序列化后的 JSON 字符串
/// * `payload`: Serialized JSON string
pub fn encode_with_namespace(ns: &str, typ: &str, payload: &str) -> String {
    if ns == "/" {
        format!("{typ}{payload}")
    } else {
        format!("{typ}{ns},{payload}")
    }
}

/// 编码并生成带 Ack ID 的文本事件报文字符串。
/// Encodes and generates a text event packet string with an Ack ID.
///
/// 格式为 `<类型>[<Namespace>,]<Ack ID><载荷>`。
/// Format is `<type>[<Namespace>,]<Ack ID><payload>`.
///
/// # 参数 (Parameters)
/// * `ns`: Namespace
/// * `typ`: 报文类型标识（例如 `"43"` 表示带 Ack ID 的报文）
/// * `typ`: Packet type identifier (e.g., `"43"` for a packet with Ack ID)
/// * `id`: 期待响应的 Ack ID 数字
/// * `id`: Expected Ack ID number for the response
/// * `payload`: 序列化后的 JSON 字符串
/// * `payload`: Serialized JSON string
pub fn encode_with_namespace_with_id(ns: &str, typ: &str, id: u64, payload: &str) -> String {
    let payload = if payload.is_empty() { "[]" } else { payload };
    if ns == "/" {
        format!("{typ}{id}{payload}")
    } else {
        format!("{typ}{ns},{id}{payload}")
    }
}

/// 编码并生成带二进制附件的事件报文头部。
/// Encodes and generates the header of an event packet with binary attachments.
///
/// 生成类似 `<类型><附件数量>-[<Namespace>,][<Ack ID>]<载荷>` 格式的字符串。
/// Generates a string formatted like `<type><number of attachments>-[<Namespace>,][<Ack ID>]<payload>`.
///
/// # 参数 (Parameters)
/// * `ns`: Namespace
/// * `typ`: 报文类型标识（通常为 `"45"`(BINARY_EVENT) 或 `"46"`(BINARY_ACK)）
/// * `typ`: Packet type identifier (usually `"45"`(BINARY_EVENT) or `"46"`(BINARY_ACK))
/// * `num_attachments`: 附件的数量（`N`）
/// * `num_attachments`: Number of attachments (`N`)
/// * `id`: 可选的 Ack ID
/// * `id`: Optional Ack ID
/// * `payload`: 包含对二进制文件引用的 JSON 载荷
/// * `payload`: JSON payload containing references to binary files
pub fn encode_binary_header(
    ns: &str,
    typ: &str,
    num_attachments: usize,
    id: Option<u64>,
    payload: &str,
) -> String {
    let mut out = String::new();
    out.push_str(typ);
    out.push_str(&num_attachments.to_string());
    out.push('-');
    if ns != "/" {
        out.push_str(ns);
        out.push(',');
    }
    if let Some(i) = id {
        out.push_str(&i.to_string());
    }
    out.push_str(payload);
    out
}

