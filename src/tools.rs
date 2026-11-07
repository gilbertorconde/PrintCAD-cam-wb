//! The tool library's editor, its import from Fusion, and the machine
//! profiles of the user's own, on the CAM preferences page.

use std::collections::BTreeMap;

use camcraft_core::job::{Holder, HolderSegment, Preset, ToolKind, ToolSpec};
use camcraft_post::profile::Profile;
use camcraft_session::library::{fusion_csv, fusion_json, system};
use printcad_bench_sdk::api::*;

use crate::panel::{button, choice, group};

/// The open dialogs the page asks for, by the id their files come back
/// under.
pub const OPEN_TOOLS: &str = "fusion-tools";
pub const OPEN_POST: &str = "machine-profile";

/// The dialog asking for a Fusion tool library.
pub fn open_tools() -> Request {
    Request::OpenFile {
        id: OPEN_TOOLS.into(),
        kinds: vec![FileKind {
            label: "Fusion tool library (CSV or JSON)".into(),
            extensions: vec!["csv".into(), "json".into()],
        }],
        multiple: true,
    }
}

/// The dialog asking for a machine profile.
pub fn open_post() -> Request {
    Request::OpenFile {
        id: OPEN_POST.into(),
        kinds: vec![FileKind {
            label: "camcraft machine profile (TOML)".into(),
            extensions: vec!["toml".into()],
        }],
        multiple: true,
    }
}

/// The dialogs asking for a setup's probe results, by the setup's id after
/// this.
pub const OPEN_PROBE: &str = "probe-results:";

/// The dialog asking for setup `setup`'s probe results.
pub fn open_probe_results(setup: &str) -> Request {
    Request::OpenFile {
        id: format!("{OPEN_PROBE}{setup}"),
        kinds: vec![FileKind {
            label: "Probe results (LinuxCNC log, CSV)".into(),
            extensions: vec!["txt".into(), "csv".into(), "log".into()],
        }],
        multiple: false,
    }
}

/// What the files picked for dialog `id` did to `tools` and `posts`, in
/// words; `None` for a dialog not the page's.
pub fn files_opened(
    id: &str,
    files: &[OpenedFile],
    tools: &mut BTreeMap<String, ToolSpec>,
    posts: &mut BTreeMap<String, String>,
) -> Option<Result<String, String>> {
    if id != OPEN_TOOLS && id != OPEN_POST {
        return None;
    }
    let mut said = Vec::new();
    for f in files {
        let Ok(text) = std::str::from_utf8(&f.contents) else {
            return Some(Err(format!("{} is not text.", f.name)));
        };
        let done = if id == OPEN_TOOLS {
            import_tools(tools, text)
        } else {
            add_post(posts, text).map(|name| format!("Added the post '{name}'."))
        };
        match done {
            Ok(s) => said.push(s),
            Err(e) => return Some(Err(format!("{}: {e}", f.name))),
        }
    }
    (!said.is_empty()).then(|| Ok(said.join(" ")))
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
/// own; what to say of it. A library exported as JSON brings each tool's
/// holder with it; one exported as CSV has none.
pub fn import_tools(tools: &mut BTreeMap<String, ToolSpec>, text: &str) -> Result<String, String> {
    let imported = if text.trim_start().starts_with('{') {
        fusion_json::import(text).map_err(|e| e.to_string())?
    } else {
        fusion_csv::import(text).map_err(|e| e.to_string())?
    };
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
const KINDS: [(ToolKind, &str); 7] = [
    (ToolKind::Flat, "Flat end mill"),
    (ToolKind::Ball, "Ball end mill"),
    (ToolKind::Bullnose, "Bull nose end mill"),
    (ToolKind::Cone, "V-bit or chamfer mill"),
    (ToolKind::Drill, "Drill"),
    (ToolKind::ThreadMill, "Thread mill"),
    (ToolKind::Probe, "Touch probe (its ball's diameter)"),
];

/// A number of a tool, by the editor's field, for showing: zero for none.
fn field(t: &ToolSpec, id: &str) -> Option<f64> {
    let opt = |v: Option<f64>| v.unwrap_or(0.0);
    Some(match id {
        "diameter" => t.diameter,
        "corner" => t.corner,
        "angle" => opt(t.angle),
        "tip_diameter" => t.tip_diameter,
        "tip_length" => t.tip_length,
        "pitch_fine" => t.pitch_range.map_or(0.0, |[f, _]| f),
        "pitch_coarse" => t.pitch_range.map_or(0.0, |[_, c]| c),
        "teeth" => f64::from(t.teeth.unwrap_or(0)),
        "flutes" => f64::from(t.flutes.unwrap_or(0)),
        "flute_length" => opt(t.flute_length),
        "shoulder_length" => opt(t.shoulder_length),
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
        "tip_length" => t.tip_length = v,
        // A range from one pitch, or to one: the other end follows it where
        // it would be passed.
        "pitch_fine" => {
            t.pitch_range = opt.map(|f| {
                let coarse = t.pitch_range.map_or(f, |[_, c]| c);
                [f, coarse.max(f)]
            })
        }
        "pitch_coarse" => {
            t.pitch_range = opt.map(|c| {
                let fine = t.pitch_range.map_or(c, |[f, _]| f);
                [fine.min(c), c]
            })
        }
        "teeth" => t.teeth = (whole > 1).then_some(whole),
        "flutes" => t.flutes = (whole > 0).then_some(whole),
        "flute_length" => t.flute_length = opt,
        "shoulder_length" => t.shoulder_length = opt,
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
    if kind == ToolKind::ThreadMill {
        out.extend([
            ("angle", "Thread angle (0: 60°)", Dim::Angle, 1),
            ("tip_length", "Tip, below the teeth", Dim::Length, 3),
            ("pitch_fine", "Finest pitch (0: any)", Dim::Length, 3),
            ("pitch_coarse", "Coarsest pitch (0: any)", Dim::Length, 3),
            ("teeth", "Rows of teeth (0: single form)", Dim::Number, 0),
        ]);
    }
    if kind == ToolKind::Cone {
        out.push(("tip_diameter", "Tip diameter", Dim::Length, 3));
    }
    // A probe has no flutes.
    if kind != ToolKind::Probe {
        out.push(("flutes", "Flutes (0: not given)", Dim::Number, 0));
    }
    out.extend([
        (
            "flute_length",
            "Flute length (0: not given)",
            Dim::Length,
            2,
        ),
        (
            "shoulder_length",
            "Shoulder length (0: not given)",
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
    rows.extend(holder_rows(t));
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

/// Which of the holder choices `t` has: none, a built-in one, or its own.
fn holder_choice(t: &ToolSpec, builtin: &[Holder]) -> usize {
    match &t.holder {
        None => 0,
        Some(h) => builtin
            .iter()
            .position(|b| b == h)
            .map_or(builtin.len() + 1, |i| i + 1),
    }
}

/// The holder's rows: which, and its segments from the tool up, each
/// editable, so a holder of its own is typed in as Fusion's is.
fn holder_rows(t: &ToolSpec) -> Vec<Widget> {
    let builtin = system::holders();
    let mut options = vec!["None".to_string()];
    options.extend(builtin.iter().map(|h| h.name.clone()));
    options.push(match &t.holder {
        Some(h) if !h.name.is_empty() && !builtin.contains(h) => format!("Its own: {}", h.name),
        _ => "Its own".into(),
    });
    let mut rows = vec![
        choice("tool/holder", "Holder", options, holder_choice(t, &builtin)),
        Widget::Text {
            text: "The stickout is how far the tool comes out of the holder. With it, the \
                   flute length and the shank, the simulator checks that nothing above the \
                   flutes meets the material."
                .into(),
            mono: false,
        },
    ];
    let Some(h) = &t.holder else {
        return rows;
    };
    rows.push(Widget::Table {
        id: "holder_segments".into(),
        columns: ["Foot Ø", "Top Ø", "Length"].map(String::from).to_vec(),
        rows: h
            .segments
            .iter()
            .map(|s| {
                vec![
                    format!("{}", s.diameter),
                    format!("{}", s.top()),
                    format!("{}", s.length),
                ]
            })
            .collect(),
        selected: None,
        editable: vec![true; 3],
    });
    rows.push(Widget::Row {
        children: vec![
            button("add_segment", "Add a segment", ButtonStyle::Secondary, true),
            button(
                "remove_segment",
                "Remove the top segment",
                ButtonStyle::Small,
                !h.segments.is_empty(),
            ),
        ],
    });
    rows
}

/// What a widget of the holder's rows did to `tool`; whether it was one.
fn holder_event(tool: &mut ToolSpec, event: &PanelEvent) -> bool {
    match event {
        PanelEvent::Choice { id, index } if id == "tool/holder" => {
            let builtin = system::holders();
            match index.checked_sub(1) {
                None => tool.holder = None,
                Some(i) => {
                    if let Some(h) = builtin.get(i) {
                        tool.holder = Some(h.clone());
                    }
                }
            }
        }
        PanelEvent::Cell {
            id,
            row,
            column,
            value,
        } if id == "holder_segments" => {
            let Some(s) = tool.holder.as_mut().and_then(|h| h.segments.get_mut(*row)) else {
                return true;
            };
            let Some(v) = value
                .trim()
                .replace(',', ".")
                .parse::<f64>()
                .ok()
                .filter(|v| v.is_finite() && *v > 0.0)
            else {
                return true;
            };
            match column {
                0 => s.diameter = v,
                1 => s.top_diameter = ((v - s.diameter).abs() > 1e-9).then_some(v),
                2 => s.length = v,
                _ => {}
            }
        }
        PanelEvent::Button { id } if id == "add_segment" => {
            let h = tool.holder.get_or_insert_with(|| Holder {
                name: String::new(),
                segments: Vec::new(),
            });
            // Wider up the holder, as most are.
            let next = h
                .segments
                .last()
                .map_or(HolderSegment::cylinder(30.0, 20.0), |s| {
                    HolderSegment::cylinder(s.top(), s.length)
                });
            h.segments.push(next);
        }
        PanelEvent::Button { id } if id == "remove_segment" => {
            if let Some(h) = &mut tool.holder {
                h.segments.pop();
                if h.segments.is_empty() {
                    tool.holder = None;
                }
            }
        }
        _ => return false,
    }
    true
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
    if holder_event(tool, event) {
        return true;
    }
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
                // A thread mill's own, which no other tool has.
                if *k != ToolKind::ThreadMill {
                    tool.tip_length = 0.0;
                    tool.pitch_range = None;
                    tool.teeth = None;
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
pub fn files_groups(posts: &BTreeMap<String, String>, chosen_post: Option<usize>) -> Vec<Widget> {
    let mut out = vec![group(
        "Import tools from Fusion",
        vec![
            Widget::Text {
                text: "In Fusion's Tool Library, right-click a library and Export it as \
                       CSV or JSON, then import that file here. JSON brings each tool's \
                       holder too."
                    .into(),
                mono: false,
            },
            button(
                "import_tools",
                "Import from Fusion…",
                ButtonStyle::Secondary,
                true,
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
    rows.push(button(
        "add_post",
        "Add a post…",
        ButtonStyle::Secondary,
        true,
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

    /// A thread mill has its tip, its pitches and its rows of teeth in the
    /// editor; a range given from one end follows it, and they go when it
    /// is made another kind.
    #[test]
    fn a_thread_mill_is_edited() {
        let mut tools = library();
        let (mut chosen, mut preset) = (Some(0), None);
        let mut send = |tools: &mut BTreeMap<String, ToolSpec>, e: PanelEvent| {
            assert!(tool_event(tools, &mut chosen, &mut preset, &e), "{e:?}");
        };
        send(
            &mut tools,
            PanelEvent::Choice {
                id: "tool/kind".into(),
                index: 5,
            },
        );
        for (field, value) in [
            ("tip_length", 0.4),
            ("pitch_coarse", 1.5),
            ("pitch_fine", 2.0),
            ("teeth", 3.0),
        ] {
            send(
                &mut tools,
                PanelEvent::Number {
                    id: format!("tool/{field}"),
                    value,
                },
            );
        }
        let t = &tools["Flat 6"];
        assert_eq!(t.kind, ToolKind::ThreadMill);
        assert_eq!(t.tip_length, 0.4);
        assert_eq!(t.pitch_range, Some([2.0, 2.0]), "the coarsest follows");
        assert_eq!(t.threads_per_turn(), 3);
        let ids: Vec<&str> = fields(t.kind).iter().map(|f| f.0).collect();
        assert!(ids.contains(&"pitch_fine") && ids.contains(&"teeth"));
        assert!(t.check().is_empty(), "{:?}", t.check());
        send(
            &mut tools,
            PanelEvent::Choice {
                id: "tool/kind".into(),
                index: 0,
            },
        );
        let t = &tools["Flat 6"];
        assert_eq!((t.tip_length, t.pitch_range, t.teeth), (0.0, None, None));
    }

    #[test]
    fn a_tool_is_given_a_holder_and_it_is_edited() {
        let mut tools = library();
        let send = |tools: &mut BTreeMap<String, ToolSpec>, e: PanelEvent| {
            let (mut chosen, mut preset) = (Some(0), None);
            assert!(tool_event(tools, &mut chosen, &mut preset, &e), "{e:?}");
        };
        send(
            &mut tools,
            PanelEvent::Number {
                id: "tool/shoulder_length".into(),
                value: 20.0,
            },
        );
        // The first built-in holder, then its foot made wider and its top a cone.
        send(
            &mut tools,
            PanelEvent::Choice {
                id: "tool/holder".into(),
                index: 1,
            },
        );
        let first = system::holders().remove(0);
        assert_eq!(tools["Flat 6"].holder.as_ref(), Some(&first));
        send(
            &mut tools,
            PanelEvent::Cell {
                id: "holder_segments".into(),
                row: 0,
                column: 1,
                value: "24".into(),
            },
        );
        send(
            &mut tools,
            PanelEvent::Cell {
                id: "holder_segments".into(),
                row: 0,
                column: 2,
                value: "nope".into(),
            },
        );
        send(
            &mut tools,
            PanelEvent::Button {
                id: "add_segment".into(),
            },
        );
        let t = &tools["Flat 6"];
        assert_eq!(t.shoulder_length, Some(20.0));
        let h = t.holder.as_ref().unwrap();
        assert_eq!(h.segments[0].top_diameter, Some(24.0));
        assert_eq!(h.segments[0].length, first.segments[0].length);
        assert_eq!(h.segments.len(), first.segments.len() + 1);
        // Changed, it is the tool's own, and the editor says so.
        assert_eq!(
            holder_choice(t, &system::holders()),
            system::holders().len() + 1
        );
        for _ in 0..h.segments.len() {
            send(
                &mut tools,
                PanelEvent::Button {
                    id: "remove_segment".into(),
                },
            );
        }
        assert_eq!(tools["Flat 6"].holder, None);
    }

    #[test]
    fn a_fusion_json_library_brings_its_holders() {
        let mut tools = library();
        let json = r#"{"data": [{"type": "flat end mill", "description": "Flat 4",
            "geometry": {"DC": 4, "LCF": 12, "LB": 25},
            "holder": {"description": "ER11", "segments": [
                {"height": 11, "lower-diameter": 16, "upper-diameter": 16}]}}]}"#;
        let said = import_tools(&mut tools, json).unwrap();
        assert!(said.starts_with("Added 1 tool(s)"), "{said}");
        let t = &tools["Flat 4"];
        assert_eq!(t.holder.as_ref().unwrap().segments[0].diameter, 16.0);
        assert_eq!(t.stickout, Some(25.0));
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

    #[test]
    fn files_picked_are_imported_by_their_dialog() {
        let (mut tools, mut posts) = (library(), BTreeMap::new());
        let csv = OpenedFile {
            name: "mill.csv".into(),
            contents: b"Type (tool_type),Diameter (tool_diameter),Description (tool_description)\nflat end mill,4,Flat 4\n".to_vec(),
        };
        let said = files_opened(OPEN_TOOLS, &[csv], &mut tools, &mut posts).unwrap();
        assert!(said.unwrap().starts_with("Added 1 tool"));
        assert_eq!(tools.len(), 2);
        let bad = OpenedFile {
            name: "x.toml".into(),
            contents: b"nope".to_vec(),
        };
        let said = files_opened(OPEN_POST, &[bad], &mut tools, &mut posts).unwrap();
        assert!(said.unwrap_err().starts_with("x.toml: "));
        assert_eq!(files_opened("other", &[], &mut tools, &mut posts), None);
    }
}
