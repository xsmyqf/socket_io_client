use serde_json::json;
use socket_io_client::{
    append_polling_packet, format_connect_packet, normalize_ns, OutgoingPacket,
};

#[test]
fn normalize_ns_works() {
    assert_eq!(normalize_ns(""), "/");
    assert_eq!(normalize_ns("/"), "/");
    assert_eq!(normalize_ns("chat"), "/chat");
    assert_eq!(normalize_ns("/chat"), "/chat");
}

#[test]
fn format_connect_packet_variants() {
    assert_eq!(format_connect_packet("/", None), "40");
    assert_eq!(format_connect_packet("/admin", None), "40/admin,");
    assert_eq!(
        format_connect_packet("/admin", Some(json!({"token":"abc"}))),
        "40/admin,{\"token\":\"abc\"}"
    );
    assert_eq!(
        format_connect_packet("/", Some(json!({"token":"abc"}))),
        "40{\"token\":\"abc\"}"
    );
}

#[test]
fn polling_body_text_and_binary() {
    let mut body = String::new();
    append_polling_packet(&OutgoingPacket::Text("42[\"a\"]".into()), &mut body);
    body.push('\u{1e}');
    append_polling_packet(&OutgoingPacket::Binary(vec![1, 2, 3, 4]), &mut body);
    assert_eq!(body, "42[\"a\"]\u{1e}bAQIDBA==");
}
