// Polygon rule areas authored in KiCad footprint libraries.
use anyhow::{bail, Context, Result};
use konnect_ipc::gen::kiapi::{board::types as board, common::types as common};
use konnect_sexp::SexpNode;
use std::collections::BTreeSet;

pub(super) fn parse_rule_area(zone: &SexpNode) -> Result<board::Zone> {
    let mut seen = BTreeSet::new();
    for child in zone.children().unwrap_or_default().iter().skip(1) {
        let tag = child
            .head()
            .context("rule area contains an unsupported atom")?;
        if !seen.insert(tag) {
            bail!("rule area contains duplicate '{tag}' clauses");
        }
        if matches!(tag, "uuid" | "tstamp" | "name" | "layer" | "min_thickness")
            && child.children().map_or(0, |c| c.len()) != 2
        {
            bail!("rule area '{tag}' must contain one value");
        }
        if !matches!(
            tag,
            "layer"
                | "layers"
                | "uuid"
                | "tstamp"
                | "name"
                | "hatch"
                | "connect_pads"
                | "min_thickness"
                | "keepout"
                | "placement"
                | "fill"
                | "polygon"
        ) {
            bail!("rule area clause '{tag}' is not supported losslessly");
        }
    }
    let keepout = zone
        .find("keepout")
        .context("only footprint keepout zones are supported")?;
    let mut settings = board::RuleAreaSettings {
        placement_source_type: board::PlacementRuleSourceType::PrstSheetName as i32,
        ..Default::default()
    };
    let mut flags = BTreeSet::new();
    for child in keepout.children().unwrap_or_default().iter().skip(1) {
        let tag = child
            .head()
            .context("keepout contains an unsupported atom")?;
        if !flags.insert(tag) {
            bail!("duplicate keepout flag '{tag}'");
        }
        if child.children().map_or(0, |c| c.len()) != 2 {
            bail!("keepout '{tag}' must have one value");
        }
        let value = match child.get(1).and_then(SexpNode::as_str) {
            Some("allowed") => false,
            Some("not_allowed") => true,
            _ => bail!("invalid keepout flag '{tag}'"),
        };
        match tag {
            "tracks" => settings.keepout_tracks = value,
            "vias" => settings.keepout_vias = value,
            "pads" => settings.keepout_pads = value,
            "copperpour" => settings.keepout_copper = value,
            "footprints" => settings.keepout_footprints = value,
            _ => bail!("unsupported keepout flag '{tag}'"),
        }
    }
    // Require all flags: KiCad's rule-area defaults differ from protobuf defaults.
    if flags.len() != 5 {
        bail!("rule area must explicitly specify all five keepout flags");
    }
    if let Some(placement) = zone.find("placement") {
        for child in placement.children().unwrap_or_default().iter().skip(1) {
            match (child.head(), child.get(1).and_then(SexpNode::as_str)) {
                (Some("enabled"), Some("no")) | (Some("sheetname"), Some("")) => {}
                _ => bail!("active or unsupported rule-area placement constraint"),
            }
        }
    }
    // KiCad writes copper-only defaults even for rule areas; they have no rule-area IPC field.
    for (tag, scalar, expected) in [
        ("connect_pads", "clearance", 0.0),
        ("fill", "thermal_gap", 0.508),
        ("fill", "thermal_bridge_width", 0.508),
        ("fill", "island_removal_mode", 0.0),
    ] {
        if let Some(parent) = zone.find(tag) {
            for child in parent.children().unwrap_or_default().iter().skip(1) {
                let name = child
                    .head()
                    .context("unsupported copper-only rule-area setting")?;
                if (tag == "connect_pads" && name != "clearance")
                    || (tag == "fill"
                        && !matches!(
                            name,
                            "thermal_gap" | "thermal_bridge_width" | "island_removal_mode"
                        ))
                {
                    bail!("unsupported rule-area '{tag}/{name}'");
                }
            }
            if let Some(value) = parent.find(scalar) {
                if !value
                    .get_f64(1)
                    .is_some_and(|v| v == expected || (expected == 0.508 && v == 0.5))
                {
                    bail!("non-default rule-area '{tag}/{scalar}' cannot be preserved");
                }
            }
        }
    }
    if let Some(thickness) = zone.find("min_thickness") {
        if !thickness
            .get_f64(1)
            .is_some_and(|v| v == 0.254 || v == 0.25)
        {
            bail!("non-default keepout min_thickness is unsupported");
        }
    }
    let mut layers = Vec::new();
    for tag in ["layer", "layers"] {
        if let Some(list) = zone.find(tag) {
            if !layers.is_empty() {
                bail!("rule area declares layer and layers");
            }
            for value in list.children().unwrap_or_default().iter().skip(1) {
                let name = value.as_str().context("invalid rule-area layer")?;
                layers.push(konnect_ipc::builders::try_layer_from_name(name)? as i32);
            }
        }
    }
    if layers.is_empty() {
        bail!("rule area has no layers");
    }
    let polygon = zone.find("polygon").context("rule area has no polygon")?;
    if polygon.children().map_or(0, |c| c.len()) != 2 {
        bail!("rule-area polygon contains unsupported clauses");
    }
    let pts = polygon
        .find("pts")
        .context("rule area polygon has no points")?;
    let mut nodes = Vec::new();
    for point in pts.children().unwrap_or_default().iter().skip(1) {
        if point.head() != Some("xy") || point.children().map_or(0, |c| c.len()) != 3 {
            bail!("only straight rule-area polygon points are supported");
        }
        let x = point.get_f64(1).context("invalid rule-area X")?;
        let y = point.get_f64(2).context("invalid rule-area Y")?;
        if !x.is_finite() || !y.is_finite() {
            bail!("rule-area coordinates must be finite");
        }
        nodes.push(common::PolyLineNode {
            geometry: Some(common::poly_line_node::Geometry::Point(
                konnect_ipc::builders::vec2(x, y),
            )),
        });
    }
    if nodes.len() < 3 {
        bail!("rule-area polygon needs at least three points");
    }
    let hatch = zone
        .find("hatch")
        .context("rule area has no hatch settings")?;
    if hatch.children().map_or(0, |c| c.len()) != 3 {
        bail!("rule-area hatch must specify style and pitch");
    }
    let style = match hatch.get(1).and_then(SexpNode::as_str) {
        Some("edge") => board::ZoneBorderStyle::ZbsDiagonalEdge,
        _ => bail!("unsupported rule-area hatch style"),
    };
    let pitch = hatch.get_f64(2).context("invalid rule-area hatch pitch")?;
    if !pitch.is_finite() || pitch <= 0.0 {
        bail!("rule-area hatch pitch must be positive");
    }
    Ok(board::Zone {
        r#type: board::ZoneType::ZtRuleArea as i32,
        layers,
        outline: Some(common::PolySet {
            polygons: vec![common::PolygonWithHoles {
                outline: Some(common::PolyLine {
                    nodes,
                    closed: true,
                }),
                holes: vec![],
            }],
        }),
        name: zone
            .find("name")
            .and_then(|n| n.get(1))
            .and_then(SexpNode::as_str)
            .unwrap_or("")
            .into(),
        settings: Some(board::zone::Settings::RuleAreaSettings(settings)),
        border: Some(board::ZoneBorderSettings {
            style: style as i32,
            pitch: Some(konnect_ipc::builders::distance(pitch)),
        }),

        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    const NATIVE: &str = include_str!("../../tests/fixtures/refresh_rule_area_10_0_6.kicad_mod");
    #[test]
    fn unsupported_or_incomplete_rule_area_evidence_refuses() {
        for source in [
            NATIVE.replace("(tracks not_allowed)", "(tracks perhaps)"),
            NATIVE.replace("(vias not_allowed)", ""),
            NATIVE.replace("(enabled no)", "(enabled yes)"),
            NATIVE.replace("(hatch edge 0.5)", "(hatch edge 0.5) (future_rule yes)"),
        ] {
            assert_ne!(source, NATIVE);
            let root = konnect_sexp::parse_sexp(&source).unwrap();
            assert!(parse_rule_area(root.find("zone").unwrap()).is_err());
        }
    }
}
