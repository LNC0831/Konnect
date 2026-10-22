//! Opt-in real KiCad validation on an independently generated disposable board.
use konnect_core::{mcp::handler::McpHandler, tools::ServerConfig};
use konnect_ipc::{gen::kiapi, KiCadIpcClient};
use prost::Message;
use serde_json::{json, Value};

fn pads(fp: &kiapi::board::types::FootprintInstance) -> Vec<(String, String)> {
    let mut result = fp
        .definition
        .iter()
        .flat_map(|d| d.items.iter())
        .filter(|i| konnect_ipc::builders::any_is(i, "kiapi.board.types.Pad"))
        .map(|i| {
            let p = kiapi::board::types::Pad::decode(i.value.as_slice()).unwrap();
            (p.number, p.net.unwrap_or_default().name)
        })
        .collect::<Vec<_>>();
    result.sort();
    result
}

#[tokio::test]
#[ignore = "requires a disposable 19-instance official-library fixture in a separate KiCad process/profile/socket"]
async fn native_refresh_retains_pose_nets_and_persists_library_rule_area() {
    let board = std::path::PathBuf::from(std::env::var("KONNECT_REFRESH_TEST_BOARD").unwrap());
    assert_eq!(board.file_name().unwrap(), "refresh-isolation.kicad_pcb");
    let socket = std::env::var("KONNECT_REFRESH_TEST_SOCKET").unwrap();
    let evidence =
        std::path::PathBuf::from(std::env::var("KONNECT_REFRESH_TEST_EVIDENCE").unwrap());
    let client = KiCadIpcClient::new(socket.clone());
    let document = client.find_open_board(&board).unwrap();
    let object_type = kiapi::common::types::KiCadObjectType::KotPcbFootprint;
    let before = client.get_items_in(document.clone(), object_type).unwrap();
    assert_eq!(before.len(), 19);
    let before_snapshot = client.save_document_to_string_in(document.clone()).unwrap();
    let handler = McpHandler::new(ServerConfig {
        kicad_cli: std::env::var("KICAD_CLI").unwrap(),
        kicad_binary: String::new(),
        ipc_address: socket,
        project_dir: None,
        jlcpcb_db_path: None,
        auto_load_toolsets: false,
        eager_toolsets: true,
    })
    .await
    .unwrap();
    let mut records = Vec::new();
    let mut id = 0;
    let call = |id, arguments| {
        handler.handle_message(json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":"update_footprints_from_library","arguments":arguments}}))
    };
    id += 1;
    let plan = call(id, json!({"board":board,"dry_run":true}))
        .await
        .unwrap()
        .result
        .unwrap();
    records.push(plan.clone());
    std::fs::write(
        evidence.join("refresh-live-responses.json"),
        serde_json::to_vec_pretty(&records).unwrap(),
    )
    .unwrap();
    let planned: Value =
        serde_json::from_str(plan["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(planned["status"], json!("ready"), "{planned}");
    assert_eq!(planned["coverage"]["changed"]["planned"], json!(19));
    assert_eq!(
        client.save_document_to_string_in(document.clone()).unwrap(),
        before_snapshot,
        "dry run must not mutate"
    );
    id += 1;
    let applied = call(
        id,
        json!({"board":board,"dry_run":false,"expected_plan_revision":planned["plan_revision"]}),
    )
    .await
    .unwrap()
    .result
    .unwrap();
    records.push(applied.clone());
    std::fs::write(
        evidence.join("refresh-live-responses.json"),
        serde_json::to_vec_pretty(&records).unwrap(),
    )
    .unwrap();
    let body: Value =
        serde_json::from_str(applied["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_ne!(applied["isError"], json!(true), "{body}");
    assert_eq!(body["status"], json!("applied"));
    let after = client.get_items_in(document.clone(), object_type).unwrap();
    assert_eq!(after.len(), before.len());
    let mut zone_count = 0;
    for item in before {
        let original =
            kiapi::board::types::FootprintInstance::decode(item.value.as_slice()).unwrap();
        let held = after
            .iter()
            .map(|i| kiapi::board::types::FootprintInstance::decode(i.value.as_slice()).unwrap())
            .find(|f| f.id == original.id)
            .unwrap();
        assert_eq!(held.position, original.position);
        assert_eq!(held.orientation, original.orientation);
        assert_eq!(held.layer, original.layer);
        assert_eq!(held.locked, original.locked);
        assert_eq!(held.symbol_path, original.symbol_path);
        assert_eq!(held.overrides, original.overrides);
        assert_eq!(pads(&held), pads(&original));
        for child in held
            .definition
            .as_ref()
            .unwrap()
            .items
            .iter()
            .filter(|i| konnect_ipc::builders::any_is(i, "kiapi.board.types.Zone"))
        {
            let zone = kiapi::board::types::Zone::decode(child.value.as_slice()).unwrap();
            assert_eq!(
                zone.r#type,
                kiapi::board::types::ZoneType::ZtRuleArea as i32
            );
            assert_eq!(zone.parent, held.id);
            zone_count += 1;
        }
    }
    assert_eq!(zone_count, 1);
    client.save_board().unwrap();
    std::fs::copy(&board, evidence.join("refresh-saved.kicad_pcb")).unwrap();
    id += 1;
    let second = call(id, json!({"board":board,"dry_run":true}))
        .await
        .unwrap()
        .result
        .unwrap();
    records.push(second.clone());
    std::fs::write(
        evidence.join("refresh-live-responses.json"),
        serde_json::to_vec_pretty(&records).unwrap(),
    )
    .unwrap();
    let second: Value =
        serde_json::from_str(second["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(
        second["status"],
        json!("noop"),
        "a completed native refresh must converge: {second}"
    );
}
