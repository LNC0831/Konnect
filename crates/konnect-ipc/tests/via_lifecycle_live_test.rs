use konnect_ipc::gen::kiapi;
use konnect_ipc::{IpcViaPlacement, KiCadIpcClient};
use prost::Message;

#[test]
#[ignore = "requires an isolated, disposable KiCad board with GND and SIGNAL nets"]
fn live_verified_via_lifecycle() {
    let board = std::path::PathBuf::from(
        std::env::var("KONNECT_VIA_TEST_BOARD").expect("scratch board required"),
    );
    assert!(
        board.file_name().unwrap() == "via-isolation.kicad_pcb",
        "never target a production board"
    );
    let socket = std::env::var("KONNECT_VIA_TEST_SOCKET").expect("isolated socket required");
    let client = KiCadIpcClient::new(socket);
    let document = client.find_open_board(&board).unwrap();
    let before = client.get_vias_in(document.clone()).unwrap();
    assert!(before.is_empty(), "start with a clean disposable fixture");
    let placed = client
        .add_via_verified_in(document.clone(), "GND", 20.0, 25.0, 0.3, 0.6)
        .unwrap();
    let IpcViaPlacement::Placed(via) = placed else {
        panic!("expected actual GND via: {placed:?}");
    };
    assert!(!via.uuid.is_empty());
    assert_eq!(via.net_name, "GND");
    let readback = client.get_vias_in(document.clone()).unwrap();
    assert_eq!(readback, vec![via.clone()]);
    assert!(client
        .delete_via_verified_in(document.clone(), &via.uuid, "stale")
        .unwrap()
        .is_none());
    assert_eq!(client.get_vias_in(document.clone()).unwrap(), readback);
    client.save_board().unwrap();
    let saved = std::fs::read_to_string(&board).unwrap();
    assert!(saved.contains(&via.uuid));
    assert!(saved.contains("GND"));
    let native = client
        .get_items_in(
            document.clone(),
            kiapi::common::types::KiCadObjectType::KotPcbVia,
        )
        .unwrap();
    let parsed = kiapi::board::types::Via::decode(native[0].value.as_slice()).unwrap();
    assert!(
        parsed.parent.is_some(),
        "capture KiCad10.0.6 native parent field"
    );
    let evidence_dir =
        std::env::var("KONNECT_VIA_TEST_EVIDENCE").expect("private evidence directory required");
    std::fs::write(
        std::path::Path::new(&evidence_dir).join("via-native.bin"),
        &native[0].value,
    )
    .unwrap();
    std::fs::write(
        std::path::Path::new(&evidence_dir).join("via-created.json"),
        serde_json::to_vec_pretty(&via).unwrap(),
    )
    .unwrap();
    assert!(client
        .delete_via_verified_in(document.clone(), &via.uuid, &via.revision)
        .unwrap()
        .is_some());
    assert!(client.get_vias_in(document.clone()).unwrap().is_empty());
    client.save_board().unwrap();
    assert!(!std::fs::read_to_string(&board).unwrap().contains(&via.uuid));
    eprintln!(
        "created/read/old-revision refusal/deleted/saved {}",
        via.uuid
    );
    // Native Undo must be checked separately on the isolated editor: no Undo IPC command exists.
}
#[test]
#[ignore = "run after one native Undo of Delete via in the isolated test editor"]
fn live_undo_restores_complete_native_via_and_save() {
    let board = std::path::PathBuf::from(std::env::var("KONNECT_VIA_TEST_BOARD").unwrap());
    assert_eq!(board.file_name().unwrap(), "via-isolation.kicad_pcb");
    let client = KiCadIpcClient::new(std::env::var("KONNECT_VIA_TEST_SOCKET").unwrap());
    let document = client.find_open_board(&board).unwrap();
    let evidence_dir =
        std::path::PathBuf::from(std::env::var("KONNECT_VIA_TEST_EVIDENCE").unwrap());
    let original = kiapi::board::types::Via::decode(
        std::fs::read(evidence_dir.join("via-native.bin"))
            .unwrap()
            .as_slice(),
    )
    .unwrap();
    let items = client
        .get_items_in(
            document.clone(),
            kiapi::common::types::KiCadObjectType::KotPcbVia,
        )
        .unwrap();
    assert_eq!(items.len(), 1);
    let restored = kiapi::board::types::Via::decode(items[0].value.as_slice()).unwrap();
    assert_eq!(
        restored, original,
        "Undo must preserve every modeled native field, including UUID/parent/padstack"
    );
    client.save_board().unwrap();
    let saved = std::fs::read_to_string(&board).unwrap();
    let tree = konnect_sexp::parse_sexp(&saved).unwrap();
    let vias = tree.find_all("via");
    assert_eq!(vias.len(), 1);
    assert_eq!(
        vias[0].find("net").unwrap().get(1).unwrap().as_str(),
        Some("GND")
    );
    std::fs::copy(&board, evidence_dir.join("undo-saved.kicad_pcb")).unwrap();
    let via = client.get_vias_in(document.clone()).unwrap().remove(0);
    std::fs::write(
        evidence_dir.join("undo-readback.json"),
        serde_json::to_vec_pretty(&via).unwrap(),
    )
    .unwrap();
    assert!(client
        .delete_via_verified_in(document, &via.uuid, &via.revision)
        .unwrap()
        .is_some());
    client.save_board().unwrap();
}
