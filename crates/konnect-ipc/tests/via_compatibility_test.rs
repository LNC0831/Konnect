use konnect_ipc::gen::kiapi;
use prost::Message;

#[test]
fn native_via_parent_and_complete_known_payload_roundtrip() {
    let payload = include_bytes!("fixtures/via_10_0_6.bin");
    let via = kiapi::board::types::Via::decode(payload.as_slice()).unwrap();
    assert!(via.parent.as_ref().is_some_and(|id| !id.value.is_empty()));
    assert_eq!(via.position.unwrap().x_nm, 20_000_000);
    assert_eq!(via.net.as_ref().unwrap().name, "GND");
    let roundtrip = kiapi::board::types::Via::decode(via.encode_to_vec().as_slice()).unwrap();
    assert_eq!(roundtrip, via);
    assert_eq!(roundtrip.parent, via.parent);
}

#[test]
fn revision_includes_native_unknown_fields_that_prost_would_drop() {
    let payload = include_bytes!("fixtures/via_10_0_6.bin");
    let mut extended = payload.to_vec();
    // Future varint field 127. Prost intentionally drops this during re-encoding.
    extended.extend_from_slice(&[0xf8, 0x07, 0x01]);
    let decoded = kiapi::board::types::Via::decode(extended.as_slice()).unwrap();
    assert_eq!(
        decoded,
        kiapi::board::types::Via::decode(payload.as_slice()).unwrap()
    );
    assert_ne!(
        konnect_ipc::client::via_revision(payload),
        konnect_ipc::client::via_revision(&extended)
    );
}
