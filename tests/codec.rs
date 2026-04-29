use socket_io_client::codec::*;

#[test]
fn ns_payload_default() {
    assert_eq!(parse_ns_payload("[\"a\"]"), ("/".to_string(), "[\"a\"]"));
}

#[test]
fn ns_payload_custom() {
    assert_eq!(
        parse_ns_payload("/chat,[\"a\"]"),
        ("/chat".to_string(), "[\"a\"]")
    );
}

#[test]
fn ns_payload_with_id_default() {
    assert_eq!(
        parse_ns_payload_with_id("12[\"a\"]"),
        ("/".to_string(), Some(12), "[\"a\"]")
    );
}

#[test]
fn ns_payload_with_id_custom() {
    assert_eq!(
        parse_ns_payload_with_id("/ns,7[\"a\"]"),
        ("/ns".to_string(), Some(7), "[\"a\"]")
    );
}

#[test]
fn binary_header() {
    assert_eq!(parse_binary_header("1-[\"a\"]"), (1, "[\"a\"]"));
    assert_eq!(parse_binary_header("2-/ns,[\"a\"]"), (2, "/ns,[\"a\"]"));
    assert_eq!(parse_binary_header("[\"a\"]"), (0, "[\"a\"]"));
}

#[test]
fn encode_default() {
    assert_eq!(encode_with_namespace("/", "42", "[\"a\"]"), "42[\"a\"]");
    assert_eq!(
        encode_with_namespace("/ns", "42", "[\"a\"]"),
        "42/ns,[\"a\"]"
    );
}

#[test]
fn encode_binary() {
    assert_eq!(
        encode_binary_header("/", "45", 1, None, "[\"a\",{\"_placeholder\":true,\"num\":0}]"),
        "451-[\"a\",{\"_placeholder\":true,\"num\":0}]"
    );
    assert_eq!(
        encode_binary_header("/ns", "46", 2, Some(13), "[\"a\"]"),
        "462-/ns,13[\"a\"]"
    );
}
