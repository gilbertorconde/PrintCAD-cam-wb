//! Tools from a Fusion tool library exported as JSON (a `.json` or `.tools`
//! library's contents), which, unlike the CSV export, carries each tool's
//! holder as its segments.
//!
//! The file is `{"data": [...]}`, a tool in each entry: its `type`, its `unit`,
//! its `geometry` under ISO 13399's short names (`DC` its diameter, `LCF` its
//! flute length, `LB` its length below the holder, `OAL` its overall length,
//! `SFDM` its shank's diameter, `NOF` its flutes, `RE` its corner radius, `SIG`
//! a drill's point angle, `TA` a chamfer mill's taper, `TP` a thread mill's
//! pitch), its `holder` with
//! `segments` of `height`, `lower-diameter` and `upper-diameter`, and its
//! presets under `start-values`. Entries that are holders alone, or tools
//! this build cannot cut with, are left out, and each says why.

use camcraft_core::job::{
    Coolant, Holder, HolderSegment, Preset, SpindleDirection, ToolKind, ToolMaterial, ToolSpec,
    free_preset_name,
};
use serde_json::Value;

use super::MM_PER_INCH;
use super::fusion_csv::{Imported, name_for};

/// A file that is not a Fusion tool library in JSON.
#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    /// Not JSON.
    #[error("not a JSON file: {0}")]
    Json(#[from] serde_json::Error),
    /// JSON, but no list of tools under `data`.
    #[error("no 'data' list: not a Fusion tool library")]
    NoData,
}

/// Read a Fusion tool library in JSON.
pub fn import(text: &str) -> Result<Imported, ImportError> {
    let root: Value = serde_json::from_str(text)?;
    let entries = root
        .get("data")
        .and_then(Value::as_array)
        .ok_or(ImportError::NoData)?;
    let mut out = Imported::default();
    for (i, entry) in entries.iter().enumerate() {
        match tool(entry) {
            Ok(tool) => {
                let name = name_for(&tool);
                out.tools.push((name, tool));
            }
            Err(why) => out.skipped.push(format!("entry {}: {why}", i + 1)),
        }
    }
    out.tools.retain(|(name, tool)| match tool.check().first() {
        Some(e) => {
            out.skipped.push(format!("'{name}': {e}"));
            false
        }
        None => true,
    });
    Ok(out)
}

/// Millimetres per unit of `unit`, as Fusion writes it.
fn scale_of(unit: Option<&str>) -> f64 {
    match unit {
        Some(u) if u.to_ascii_lowercase().starts_with("inch") => MM_PER_INCH,
        _ => 1.0,
    }
}

fn number(v: &Value, key: &str) -> Option<f64> {
    let n = v.get(key)?;
    n.as_f64()
        .or_else(|| n.as_str().and_then(|s| s.trim().parse().ok()))
        .filter(|x: &f64| x.is_finite())
}

fn text(v: &Value, key: &str) -> Option<String> {
    let s = v.get(key)?.as_str()?.trim();
    (!s.is_empty()).then(|| s.to_string())
}

fn whole(v: &Value, key: &str) -> Option<u32> {
    let n = number(v, key)?.round();
    // Rounded, and in range, first.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    (n >= 1.0 && n <= f64::from(u32::MAX)).then_some(n as u32)
}

/// One entry as a tool.
fn tool(entry: &Value) -> Result<ToolSpec, String> {
    let kind_text = text(entry, "type").unwrap_or_default().to_ascii_lowercase();
    let kind = match kind_text.as_str() {
        "flat end mill" => ToolKind::Flat,
        "ball end mill" => ToolKind::Ball,
        "bull nose end mill" => ToolKind::Bullnose,
        "chamfer mill" | "spot drill" => ToolKind::Cone,
        "drill" => ToolKind::Drill,
        "thread mill" => ToolKind::ThreadMill,
        "holder" => return Err("a holder, not a tool".into()),
        "" => return Err("no tool type".into()),
        other => return Err(format!("'{other}' tools are not supported yet")),
    };
    let scale = scale_of(entry.get("unit").and_then(Value::as_str));
    let geometry = entry.get("geometry").cloned().unwrap_or(Value::Null);
    let length = |key: &str| {
        number(&geometry, key)
            .filter(|v| *v > 0.0)
            .map(|v| v * scale)
    };
    let diameter = length("DC").ok_or("no diameter")?;
    let presets = entry
        .get("start-values")
        .and_then(|s| s.get("presets"))
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    let mut tool = ToolSpec {
        kind,
        diameter,
        corner: match kind {
            ToolKind::Bullnose => length("RE").unwrap_or(0.0),
            _ => 0.0,
        },
        angle: match kind_text.as_str() {
            "chamfer mill" => number(&geometry, "TA").map(|a| 2.0 * a),
            "spot drill" | "drill" => number(&geometry, "SIG"),
            "thread mill" => number(&geometry, "thread-profile-angle"),
            _ => None,
        },
        tip_diameter: match kind {
            ToolKind::Cone => length("tip-diameter").unwrap_or(0.0),
            _ => 0.0,
        },
        tip_length: 0.0,
        // Fusion's thread mill has the one pitch it cuts.
        pitch_range: match kind {
            ToolKind::ThreadMill => length("TP")
                .or_else(|| length("thread-pitch"))
                .map(|p| [p, p]),
            _ => None,
        },
        teeth: None,
        number: entry
            .get("post-process")
            .and_then(|p| whole(p, "number"))
            .unwrap_or(1),
        rpm: presets.first().and_then(|p| whole(p, "n")).unwrap_or(18000),
        comment: text(entry, "description").unwrap_or_default(),
        flutes: whole(&geometry, "NOF"),
        flute_length: length("LCF"),
        shoulder_length: length("shoulder-length"),
        shank_diameter: length("SFDM"),
        overall_length: length("OAL"),
        stickout: length("LB"),
        material: match text(entry, "BMC").map(|m| m.to_ascii_lowercase()) {
            Some(m) if m.contains("carbide") => Some(ToolMaterial::Carbide),
            Some(m) if m.contains("hss") || m.contains("high speed") => Some(ToolMaterial::Hss),
            _ => None,
        },
        vendor: text(entry, "vendor"),
        product: text(entry, "product-id"),
        holder: entry.get("holder").and_then(|h| holder(h, scale)),
        spindle: SpindleDirection::Clockwise,
        coolant: Coolant::Off,
        presets: Vec::new(),
    };
    for (i, p) in presets.iter().enumerate() {
        let preset = preset(p, i, scale);
        let name = free_preset_name(&tool.presets, &preset.name);
        tool.presets.push(Preset { name, ..preset });
    }
    Ok(tool)
}

/// A holder's segments, from the tool upward, in millimetres; none when it
/// has no segment that is one.
fn holder(h: &Value, tool_scale: f64) -> Option<Holder> {
    let scale = h
        .get("unit")
        .and_then(Value::as_str)
        .map_or(tool_scale, |u| scale_of(Some(u)));
    let segments: Vec<HolderSegment> = h
        .get("segments")?
        .as_array()?
        .iter()
        .filter_map(|s| {
            let length = number(s, "height")? * scale;
            let lower = number(s, "lower-diameter")? * scale;
            let upper = number(s, "upper-diameter").map_or(lower, |u| u * scale);
            (length > 0.0 && lower > 0.0 && upper > 0.0).then_some(HolderSegment {
                diameter: lower,
                length,
                top_diameter: ((upper - lower).abs() > 1e-9).then_some(upper),
            })
        })
        .collect();
    (!segments.is_empty()).then(|| Holder {
        name: text(h, "description").unwrap_or_default(),
        segments,
    })
}

/// A preset; `index` names one Fusion left unnamed.
fn preset(p: &Value, index: usize, scale: f64) -> Preset {
    let feed = |key: &str| number(p, key).filter(|v| *v > 0.0).map(|v| v * scale);
    let used = |key: &str| p.get(key).and_then(Value::as_bool).unwrap_or(false);
    Preset {
        name: text(p, "name").unwrap_or_else(|| format!("Preset {}", index + 1)),
        comment: String::new(),
        rpm: whole(p, "n"),
        feed: feed("v_f"),
        plunge_feed: feed("v_f_plunge"),
        ramp_feed: feed("v_f_ramp"),
        stepdown: used("use-stepdown").then(|| feed("stepdown")).flatten(),
        stepover: used("use-stepover").then(|| feed("stepover")).flatten(),
        stock_to_leave: None,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
mod tests {
    use super::*;

    const LIBRARY: &str = r#"{
      "data": [
        {
          "type": "flat end mill", "unit": "millimeters",
          "description": "6 mm 3 flute", "product-id": "FEM-6", "BMC": "carbide",
          "geometry": {"DC": 6, "LCF": 18, "LB": 30, "OAL": 60, "SFDM": 6, "NOF": 3,
                       "shoulder-length": 20},
          "holder": {"description": "BT30 ER20", "unit": "millimeters",
                     "segments": [{"height": 20, "lower-diameter": 32, "upper-diameter": 32},
                                  {"height": 30, "lower-diameter": 34, "upper-diameter": 40}]},
          "post-process": {"number": 4},
          "start-values": {"presets": [{"name": "Aluminium", "n": 12000, "v_f": 900,
                                        "v_f_plunge": 300, "stepdown": 2,
                                        "use-stepdown": true}]}
        },
        {"type": "holder", "segments": []},
        {"type": "slot mill", "geometry": {"DC": 6}},
        {
          "type": "ball end mill", "unit": "inches", "description": "1/8 ball",
          "geometry": {"DC": 0.125, "LCF": 0.5}
        }
      ]
    }"#;

    #[test]
    fn a_tool_comes_with_its_holder() {
        let got = import(LIBRARY).unwrap();
        assert_eq!(got.tools.len(), 2, "{:?}", got.skipped);
        assert_eq!(got.skipped.len(), 2, "{:?}", got.skipped);
        let (name, t) = &got.tools[0];
        assert_eq!(name, "FEM-6");
        assert_eq!(t.number, 4);
        assert_eq!(t.flute_length, Some(18.0));
        assert_eq!(t.shoulder_length, Some(20.0));
        assert_eq!(t.stickout, Some(30.0));
        assert_eq!(t.material, Some(ToolMaterial::Carbide));
        let h = t.holder.as_ref().unwrap();
        assert_eq!(h.name, "BT30 ER20");
        assert_eq!(h.segments[0], HolderSegment::cylinder(32.0, 20.0));
        assert_eq!(h.segments[1].top_diameter, Some(40.0));
        assert_eq!(t.gauge_length(), Some(80.0));
        assert_eq!(t.rpm, 12000);
        assert_eq!(t.presets[0].stepdown, Some(2.0));
        assert_eq!(t.presets[0].stepover, None);
        let (_, ball) = &got.tools[1];
        assert!((ball.diameter - 3.175).abs() < 1e-9);
        assert!((ball.flute_length.unwrap() - 12.7).abs() < 1e-9);
        assert!(import("[1, 2]").is_err());
        assert!(import("nope").is_err());
    }
}
