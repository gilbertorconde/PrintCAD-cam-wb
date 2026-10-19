//! The tool library's editor, its import from Fusion, and the machine
//! profiles of the user's own, on the CAM preferences page.
//!
//! A package in printCAD's sandbox reads no files, and printCAD offers it no
//! dialog to open one, so a file is named by its path and the package's
//! helper reads it ([`READ`]).

use std::collections::BTreeMap;

use cam_protocol::{HELPER, Request, Response, VERSION};
use camcraft_core::job::{Preset, ToolKind, ToolSpec};
use camcraft_post::profile::Profile;
use camcraft_session::library::fusion_csv;
use printcad_bench_sdk::api::*;
use printcad_bench_sdk::{host, serde_json};

use crate::panel::{button, choice, group};

/// The job that reads a file through the helper: its input is the path.
pub const READ: &str = "read";

/// What a file being read is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reading {
    /// A Fusion tool library, exported as CSV, to add to the library.
    Tools,
    /// A machine profile, as camcraft's TOML.
    Post,
}

/// What the [`READ`] job does: ask the helper for the file at `path`.
pub fn read_job(path: &str) -> Result<String, String> {
    let request = Request {
        version: VERSION,
        read: Some(path.to_string()),
        ..Request::default()
    };
    let input = serde_json::to_string(&request).map_err(|e| e.to_string())?;
    let out = host::helper(HELPER, input.as_bytes())?;
    String::from_utf8(out).map_err(|_| "the helper's answer is not text".into())
}

/// The text a [`READ`] job's result holds, or why there is none.
pub fn read_text(result: &Result<String, String>) -> Result<String, String> {
    let answer = result.as_ref().map_err(Clone::clone)?;
    match serde_json::from_str(answer).map_err(|e| format!("the helper's answer: {e}"))? {
        Response::Read { text } => Ok(text),
        Response::Refused { why } => Err(why),
        Response::Ran(_) => Err("the helper ran a setup instead".into()),
    }
}

/// `base`, or `base 2`, `base 3`… where `taken` already has it.
fn free_name(base: &str, taken: impl Fn(&str) -> bool) -> String {
    let mut name = base.to_string();
    let mut n = 2;
    while taken(&name) {
        name = format!("{base} {n}");
        n += 1;
    }
    name
}

/// A Fusion tool library's tools added to `tools`, each under a name of its
/// own; what to say of it.
pub fn import_tools(tools: &mut BTreeMap<String, ToolSpec>, text: &str) -> Result<String, String> {
    let imported = fusion_csv::import(text).map_err(|e| e.to_string())?;
    let count = imported.tools.len();
    for (name, tool) in imported.tools {
        let name = free_name(&name, |n| tools.contains_key(n));
        tools.insert(name, tool);
    }
    let mut said = format!("Added {count} tool(s) from Fusion.");
    if !imported.skipped.is_empty() {
        said.push_str(&format!(
            " Left out {}: {}",
            imported.skipped.len(),
            imported.skipped.join("; ")
        ));
    }
    Ok(said)
}

/// A machine profile kept by its own name, which a built-in one may not
/// have; that name.
pub fn add_post(posts: &mut BTreeMap<String, String>, text: &str) -> Result<String, String> {
    let profile = Profile::from_toml(text).map_err(|e| format!("not a machine profile: {e}"))?;
    let name = profile.name.trim().to_string();
    if name.is_empty() {
        return Err("the profile has no name".into());
    }
    if Profile::builtin(&name).is_some() {
        return Err(format!("'{name}' is a built-in post; rename the profile"));
    }
    posts.insert(name.clone(), text.to_string());
    Ok(name)
}

/// The tool kinds, as the editor lists them.
const KINDS: [(ToolKind, &str); 5] = [
    (ToolKind::Flat, "Flat end mill"),
    (ToolKind::Ball, "Ball end mill"),
    (ToolKind::Bullnose, "Bull nose end mill"),
    (ToolKind::Cone, "V-bit or chamfer mill"),
    (ToolKind::Drill, "Drill"),
];

/// A number of a tool, by the editor's field, for showing: zero for none.
fn field(t: &ToolSpec, id: &str) -> Option<f64> {
    let opt = |v: Option<f64>| v.unwrap_or(0.0);
    Some(match id {
        "diameter" => t.diameter,
        "corner" => t.corner,
        "angle" => opt(t.angle),
        "tip_diameter" => t.tip_diameter,
        "flutes" => f64::from(t.flutes.unwrap_or(0)),
        "flute_length" => opt(t.flute_length),
        "shank_diameter" => opt(t.shank_diameter),
        "overall_length" => opt(t.overall_length),
        "stickout" => opt(t.stickout),
        "number" => f64::from(t.number),
        "rpm" => f64::from(t.rpm),
        _ => return None,
    })
}

/// Set a number of a tool, if it makes sense for it; zero clears one that
/// need not be given. Whether it took.
fn set_field(t: &mut ToolSpec, id: &str, v: f64) -> bool {
    if !v.is_finite() || v < 0.0 {
        return false;
    }
    let opt = (v > 0.0).then_some(v);
    let whole = v.round() as u32;
    match id {
        "diameter" if v > 0.0 && t.corner <= v / 2.0 => t.diameter = v,
        "corner" if v <= t.diameter / 2.0 => t.corner = v,
        "angle" if v < 180.0 => t.angle = opt,
        "tip_diameter" if v < t.diameter => t.tip_diameter = v,
        "flutes" => t.flutes = (whole > 0).then_some(whole),
        "flute_length" => t.flute_length = opt,
        "shank_diameter" => t.shank_diameter = opt,
        "overall_length" => t.overall_length = opt,
        "stickout" => t.stickout = opt,
        "number" if whole >= 1 => t.number = whole,
        "rpm" => t.rpm = whole,
        _ => return false,
    }
    true
}

/// The editor's fields for a tool of `kind`, with their labels.
fn fields(kind: ToolKind) -> Vec<(&'static str, &'static str, Dim, usize)> {
    let mut out = vec![("diameter", "Diameter", Dim::Length, 3)];
    if kind == ToolKind::Bullnose {
        out.push(("corner", "Corner radius", Dim::Length, 3));
    }
    if matches!(kind, ToolKind::Cone | ToolKind::Drill) {
        out.push(("angle", "Included angle (0: none)", Dim::Angle, 1));
    }
    if kind == ToolKind::Cone {
        out.push(("tip_diameter", "Tip diameter", Dim::Length, 3));
    }
    out.extend([
        ("flutes", "Flutes (0: not given)", Dim::Number, 0),
        (
            "flute_length",
            "Flute length (0: not given)",
            Dim::Length,
            2,
        ),
        (
            "shank_diameter",
            "Shank diameter (0: not given)",
            Dim::Length,
            2,
        ),
        (
            "overall_length",
            "Overall length (0: not given)",
            Dim::Length,
            2,
        ),
        ("stickout", "Stickout (0: not given)", Dim::Length, 2),
        ("number", "Tool number", Dim::Number, 0),
        ("rpm", "Spindle, rev/min", Dim::Number, 0),
    ]);
    out
}

fn unbound(id: String, label: &str, value: f64, dim: Dim, decimals: usize) -> Widget {
    Widget::Number {
        id,
        label: label.into(),
        value,
        dim,
        bind: None,
        min: None,
        max: None,
        decimals,
        error: None,
    }
}

fn cell(v: Option<f64>) -> String {
    v.map_or(String::new(), |v| format!("{v}"))
}

/// The chosen tool's editor: its name, kind, dimensions and presets.
pub fn tool_editor(name: &str, t: &ToolSpec, chosen_preset: Option<usize>) -> Widget {
    let mut rows = vec![
        Widget::TextField {
            id: "tool/name".into(),
            label: "Name".into(),
            value: name.into(),
        },
        choice(
            "tool/kind",
            "Kind",
            KINDS.iter().map(|(_, l)| l.to_string()).collect(),
            KINDS.iter().position(|(k, _)| *k == t.kind).unwrap_or(0),
        ),
    ];
    for (id, label, dim, decimals) in fields(t.kind) {
        rows.push(unbound(
            format!("tool/{id}"),
            label,
            field(t, id).unwrap_or(0.0),
            dim,
            decimals,
        ));
    }
    rows.push(Widget::TextField {
        id: "tool/comment".into(),
        label: "Comment".into(),
        value: t.comment.clone(),
    });
    rows.push(Widget::Table {
        id: "presets".into(),
        columns: [
            "Preset", "rev/min", "Feed", "Plunge", "Stepover", "Stepdown",
        ]
        .map(String::from)
        .to_vec(),
        rows: t
            .presets
            .iter()
            .map(|p| {
                vec![
                    p.name.clone(),
                    cell(p.rpm.map(f64::from)),
                    cell(p.feed),
                    cell(p.plunge_feed),
                    cell(p.stepover),
                    cell(p.stepdown),
                ]
            })
            .collect(),
        selected: chosen_preset,
        editable: vec![true; 6],
    });
    rows.push(Widget::Row {
        children: vec![
            button("add_preset", "Add a preset", ButtonStyle::Secondary, true),
            button(
                "remove_preset",
                "Remove the chosen preset",
                ButtonStyle::Small,
                chosen_preset.is_some(),
            ),
        ],
    });
    group("The chosen tool", rows)
}

/// What a widget of the tool editor did to the library. Whether it was one.
pub fn tool_event(
    tools: &mut BTreeMap<String, ToolSpec>,
    chosen: &mut Option<usize>,
    chosen_preset: &mut Option<usize>,
    event: &PanelEvent,
) -> bool {
    let Some(name) = chosen.and_then(|i| tools.keys().nth(i).cloned()) else {
        return false;
    };
    let Some(tool) = tools.get_mut(&name) else {
        return false;
    };
    match event {
        PanelEvent::Number { id, value } => match id.strip_prefix("tool/") {
            Some(f) => {
                set_field(tool, f, *value);
            }
            None => return false,
        },
        PanelEvent::Choice { id, index } if id == "tool/kind" => {
            if let Some((k, _)) = KINDS.get(*index) {
                tool.kind = *k;
                if *k != ToolKind::Bullnose {
                    tool.corner = 0.0;
                }
            }
        }
        PanelEvent::Text { id, value } if id == "tool/comment" => tool.comment = value.clone(),
        PanelEvent::Text { id, value } if id == "tool/name" => {
            let to = value.trim();
            if to.is_empty() || to == name || tools.contains_key(to) {
                return true;
            }
            if let Some(t) = tools.remove(&name) {
                tools.insert(to.to_string(), t);
                *chosen = tools.keys().position(|k| k == to);
            }
        }
        PanelEvent::Select { id, index } if id == "presets" => *chosen_preset = Some(*index),
        PanelEvent::Cell {
            id,
            row,
            column,
            value,
        } if id == "presets" => {
            let Some(p) = tool.presets.get_mut(*row) else {
                return true;
            };
            let text = value.trim();
            let number = text.replace(',', ".").parse::<f64>().ok();
            let given = |n: Option<f64>| n.filter(|v| v.is_finite() && *v > 0.0);
            match column {
                0 if !text.is_empty() => p.name = text.to_string(),
                1 => p.rpm = given(number).map(|v| v.round() as u32),
                2 => p.feed = given(number),
                3 => p.plunge_feed = given(number),
                4 => p.stepover = given(number),
                5 => p.stepdown = given(number),
                _ => {}
            }
        }
        PanelEvent::Button { id } if id == "add_preset" => {
            let mut p = chosen_preset
                .and_then(|i| tool.presets.get(i).cloned())
                .unwrap_or_else(|| Preset {
                    name: String::new(),
                    comment: String::new(),
                    rpm: Some(tool.rpm),
                    feed: None,
                    plunge_feed: None,
                    ramp_feed: None,
                    stepdown: None,
                    stepover: None,
                    stock_to_leave: None,
                });
            let base = if p.name.is_empty() {
                "Preset".to_string()
            } else {
                p.name.clone()
            };
            p.name = free_name(&base, |n| tool.presets.iter().any(|q| q.name == n));
            tool.presets.push(p);
            *chosen_preset = Some(tool.presets.len() - 1);
        }
        PanelEvent::Button { id } if id == "remove_preset" => {
            if let Some(i) = chosen_preset.take()
                && i < tool.presets.len()
            {
                tool.presets.remove(i);
            }
        }
        _ => return false,
    }
    true
}

/// The groups for importing tools and keeping posts.
pub fn files_groups(
    posts: &BTreeMap<String, String>,
    import_path: &str,
    post_path: &str,
    chosen_post: Option<usize>,
    reading: bool,
) -> Vec<Widget> {
    let mut out = vec![group(
        "Import tools from Fusion",
        vec![
            Widget::Text {
                text: "In Fusion's Tool Library, right-click a library and Export it as \
                       CSV, then give the file's full path here."
                    .into(),
                mono: false,
            },
            Widget::TextField {
                id: "import_path".into(),
                label: "CSV file".into(),
                value: import_path.into(),
            },
            button(
                "import_tools",
                "Import the tools",
                ButtonStyle::Secondary,
                !reading && !import_path.trim().is_empty(),
            ),
        ],
    )];
    let mut rows = vec![Widget::Text {
        text: "A machine profile of your own, in camcraft's TOML (its README says how to \
               write one), joins the built-in posts. A setup posting with it carries a copy."
            .into(),
        mono: false,
    }];
    if !posts.is_empty() {
        rows.push(Widget::Table {
            id: "posts".into(),
            columns: vec!["Your posts".into()],
            rows: posts.keys().map(|n| vec![n.clone()]).collect(),
            selected: chosen_post,
            editable: Vec::new(),
        });
        rows.push(button(
            "remove_post",
            "Remove the chosen post",
            ButtonStyle::Small,
            chosen_post.is_some(),
        ));
    }
    rows.push(Widget::TextField {
        id: "post_path".into(),
        label: "Profile file".into(),
        value: post_path.into(),
    });
    rows.push(button(
        "add_post",
        "Add the post",
        ButtonStyle::Secondary,
        !reading && !post_path.trim().is_empty(),
    ));
    out.push(group("Posts", rows));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn library() -> BTreeMap<String, ToolSpec> {
        BTreeMap::from([(
            "Flat 6".to_string(),
            ToolSpec {
                diameter: 6.0,
                ..ToolSpec::default()
            },
        )])
    }

    #[test]
    fn a_tool_is_edited_and_renamed() {
        let mut tools = library();
        let (mut chosen, mut preset) = (Some(0), None);
        let mut send = |e: PanelEvent| {
            assert!(
                tool_event(&mut tools, &mut chosen, &mut preset, &e),
                "{e:?}"
            );
        };
        send(PanelEvent::Choice {
            id: "tool/kind".into(),
            index: 2,
        });
        send(PanelEvent::Number {
            id: "tool/corner".into(),
            value: 1.0,
        });
        // More than the radius does not take.
        send(PanelEvent::Number {
            id: "tool/corner".into(),
            value: 4.0,
        });
        send(PanelEvent::Number {
            id: "tool/flute_length".into(),
            value: 22.0,
        });
        send(PanelEvent::Text {
            id: "tool/name".into(),
            value: "Bull 6".into(),
        });
        send(PanelEvent::Button {
            id: "add_preset".into(),
        });
        send(PanelEvent::Cell {
            id: "presets".into(),
            row: 0,
            column: 2,
            value: "1500".into(),
        });
        let t = &tools["Bull 6"];
        assert_eq!(t.kind, ToolKind::Bullnose);
        assert_eq!(t.corner, 1.0);
        assert_eq!(t.flute_length, Some(22.0));
        assert_eq!(t.presets[0].name, "Preset");
        assert_eq!(t.presets[0].feed, Some(1500.0));
        assert_eq!(chosen, Some(0));
        assert!(!tools.contains_key("Flat 6"));
    }

    #[test]
    fn a_fusion_library_joins_under_names_of_its_own() {
        let mut tools = library();
        let csv = "Number (tool_number),Type (tool_type),Diameter (tool_diameter),Description (tool_description)\n\
                   1,flat end mill,6,Flat 6\n\
                   2,ball end mill,3,Ball 3\n\
                   3,laser cutter,1,Laser\n";
        let said = import_tools(&mut tools, csv).unwrap();
        assert!(said.starts_with("Added 2 tool(s)"), "{said}");
        assert!(said.contains("Left out 1"), "{said}");
        assert_eq!(tools.len(), 3, "{:?}", tools.keys().collect::<Vec<_>>());
        assert!(import_tools(&mut tools, "a,b\n1,2\n").is_err());
    }

    #[test]
    fn a_post_of_ones_own_is_kept_by_its_name() {
        let grbl = include_str!("../camcraft/crates/camcraft-post/profiles/grbl.toml");
        let mine = grbl.replacen("\"GRBL\"", "\"My mill\"", 1);
        let mut posts = BTreeMap::new();
        assert_eq!(add_post(&mut posts, &mine).unwrap(), "My mill");
        assert!(posts.contains_key("My mill"));
        assert!(add_post(&mut posts, grbl).unwrap_err().contains("built-in"));
        assert!(add_post(&mut posts, "not toml").is_err());
    }
}
