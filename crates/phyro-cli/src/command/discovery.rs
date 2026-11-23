use super::{edit, json, Value, MAX_BATCH_BYTES};

pub const HELP: &str = "\
Phyro CLI — edit and export native drawings without opening the desktop editor

  phyro-cli describe
  phyro-cli mcp
  phyro-cli new --out NEW.vanim [--width W] [--height H] [--frames N] [--fps N]
  phyro-cli inspect PROJECT [--clip ID] [--frame T]
  phyro-cli lookup PROJECT (--name NAME | --object-id ID) [--clip ID] [--frame T]
  phyro-cli preview PROJECT --clip ID --frame T --out FILE.png [--scale N]
  phyro-cli apply PROJECT --batch FILE.json|- --out NEW.vanim
  phyro-cli export PROJECT --clip ID --out NEW_DIRECTORY [--format sheet|frames|sheet-and-frames|packed] [--columns N] [--scale N]
  phyro-cli export PROJECT (--clip ID | --set ID) --out NEW_DIRECTORY --format png-sequence|separate-sheets|shared-sheet [--layout grid|packed] [--columns N] [--scale N]
  phyro-cli export PROJECT --set ID --out NEW_DIRECTORY [--scale N]
  phyro-cli export-artboard PROJECT --clip ID --frame T --out PATH [--format png|svg|both] [--scale N]
  phyro-cli export-selection PROJECT --clip ID --frame T --object-ids ID[,ID...] --out PATH [--mode combined|individual] [--format png|svg|both] [--scale N]
  phyro-cli export-selection-animation PROJECT --clip ID --frame REF --object-ids ID[,ID...] --out NEW_DIRECTORY [--from FIRST] [--to LAST] [--format sheet|frames|sheet-and-frames] [--columns N] [--scale N]

Start with describe for versioned JSON describing every command and edit operation.
All project commands write one JSON response to stdout, including on error.
Use --scale 0.256 to render PNGs at 25.6% of their native size. Named sheet layouts default to the project preset.
Frame 0 is the first frame. A drawing holds from exposure_start until the next exposure.
IDs for artwork and layers belong to that drawing. Positions use artboard pixels.

Example with the bundled character:
  phyro-cli lookup lantern-walk.vanim --name \"Sleeve fabric\" --clip 1 --frame 0
  phyro-cli apply lantern-walk.vanim --batch edits.json --out walk-edited.vanim
  phyro-cli preview walk-edited.vanim --clip 1 --frame 4 --out pose.png
  phyro-cli export-selection walk-edited.vanim --clip 1 --frame 4 --object-ids 20 --out sleeve.png
  phyro-cli export-selection walk-edited.vanim --clip 1 --frame 4 --object-ids 7,20 --mode individual --format svg --out vector-parts
  phyro-cli export-selection-animation walk-edited.vanim --clip 1 --frame 0 --object-ids 20 --out sleeve-walk --columns 4
  phyro-cli export walk-edited.vanim --clip 1 --out walk-sheet --format sheet --columns 4

In edits.json, use version 1, the revision from lookup, and an operations array.
Use describe for complete examples. Choose a fresh output path for every write.";

pub(super) struct Spec {
    name: &'static str,
    pub usage: &'static str,
    pub options: &'static [&'static str],
    pub required: &'static [&'static str],
    result: &'static str,
    result_fields: &'static [&'static str],
}

impl Spec {
    pub(super) fn reads_project(&self) -> bool {
        !matches!(self.name, "new" | "describe")
    }
}

const COMMANDS: &[Spec] = &[
    Spec {
        name: "new",
        usage: "phyro-cli new --out NEW.vanim [--width W] [--height H] [--frames N] [--fps N]",
        options: &[
            "out",
            "width",
            "height",
            "frames",
            "fps",
        ],
        required: &["out"],
        result: "Create a blank editable project. Inspect the new file before applying a batch.",
        result_fields: &[
            "output",
            "clip_id",
            "exposure_start",
            "layer_id",
            "artboard",
            "frames",
            "fps",
        ],
    },
    Spec {
        name: "describe",
        usage: "phyro-cli describe",
        options: &[],
        required: &[],
        result: "Discover the versioned command contract and edit operations.",
        result_fields: &[
            "commands",
            "options",
            "batch",
            "operations",
            "examples",
        ],
    },
    Spec {
        name: "inspect",
        usage: "phyro-cli inspect PROJECT [--clip ID] [--frame T]",
        options: &["clip", "frame"],
        required: &[],
        result: "List clips, artboards and held exposures. Pass a frame to inspect the editable drawing, its layers, objects and bones.",
        result_fields: &["coordinates", "animation_sets", "clips"],
    },
    Spec {
        name: "lookup",
        usage: "phyro-cli lookup PROJECT (--name NAME | --object-id ID) [--clip ID] [--frame T]",
        options: &["name", "object-id", "clip", "frame"],
        required: &[],
        result: "Resolve a single object. Add clip and frame to distinguish matches reported by ambiguous_object.",
        result_fields: &[
            "clip_id",
            "exposure_start",
            "exposure_duration",
            "layer_id",
            "object_id",
            "group_id",
            "name",
            "kind",
            "keyframes",
        ],
    },
    Spec {
        name: "preview",
        usage: "phyro-cli preview PROJECT --clip ID --frame T --out FILE.png [--scale N]",
        options: &["clip", "frame", "out", "scale"],
        required: &["clip", "frame", "out"],
        result: "Render one frame of the active drawing as transparent PNG.",
        result_fields: &[
            "output",
            "files",
            "clip_id",
            "frame",
            "exposure_start",
            "exposure_duration",
            "fps",
            "frame_size",
            "pivot",
            "scale",
        ],
    },
    Spec {
        name: "apply",
        usage: "phyro-cli apply PROJECT --batch FILE.json|- --out NEW.vanim",
        options: &["batch", "out"],
        required: &["batch", "out"],
        result: "Apply the whole batch and save one new editable project. Inspect the new file to obtain its own revision.",
        result_fields: &["output", "operations"],
    },
    Spec {
        name: "export",
        usage: "phyro-cli export PROJECT (--clip ID | --set ID) --out NEW_DIRECTORY [--format FORMAT] [--layout grid|packed] [--columns N] [--scale N]",
        options: &[
            "clip",
            "set",
            "out",
            "format",
            "layout",
            "columns",
            "scale",
        ],
        required: &["out"],
        result: "Choose exactly one clip or animation set. A clip includes its holds. Choose png-sequence, separate-sheets or shared-sheet for either scope. Set exports use each clip’s saved include_in_set_export setting. Direct clip exports include the chosen clip. Named sheets use the project layout unless --layout is supplied. Each requested sheet is one PNG. Nested artwork keeps its timing in each export.",
        result_fields: &[
            "output",
            "files",
            "clip_id",
            "set_id",
            "set_name",
            "clip_ids",
            "format",
            "layout",
            "frame_count",
            "fps",
            "scale",
            "frame_size",
            "pivot",
            "metadata",
        ],
    },
    Spec {
        name: "export-artboard",
        usage: "phyro-cli export-artboard PROJECT --clip ID --frame T --out PATH [--format png|svg|both] [--scale N]",
        options: &["clip", "frame", "out", "format", "scale"],
        required: &["clip", "frame", "out"],
        result: "Export the current artboard at its canvas size. Use a new file for PNG or SVG, or a fresh directory for both. Scale applies to PNG.",
        result_fields: &["output","files","assets","mode","format","scale","clip_id","frame","exposure_start","exposure_duration"],
    },
    Spec {
        name: "export-selection",
        usage: "phyro-cli export-selection PROJECT --clip ID --frame T --object-ids ID[,ID...] --out PATH [--mode combined|individual] [--format png|svg|both] [--scale N]",
        options: &["clip", "frame", "object-ids", "out", "mode", "format", "scale"],
        required: &["clip", "frame", "object-ids", "out"],
        result: "Export selected artwork at one frame, including off-artboard artwork. Combined crops to the artwork bounds. Individual writes one named image per top-level object or group. Both formats and all individual exports write to a fresh directory. Each asset reports its absolute path, expanded object_ids, signed top-left origin and size. PNG uses output pixels at the requested scale. SVG uses native artwork coordinates at scale 1.",
        result_fields: &[
            "output", "files", "assets", "mode", "format", "scale", "clip_id", "frame",
            "exposure_start", "exposure_duration",
        ],
    },
    Spec {
        name: "export-selection-animation",
        usage: "phyro-cli export-selection-animation PROJECT --clip ID --frame REF --object-ids ID[,ID...] --out NEW_DIRECTORY [--from FIRST] [--to LAST] [--format sheet|frames|sheet-and-frames] [--columns N] [--scale N]",
        options: &["clip", "frame", "object-ids", "out", "from", "to", "format", "columns", "scale"],
        required: &["clip", "frame", "object-ids", "out"],
        result: "Export the selected objects or groups across an inclusive frame range. IDs belong to the drawing at --frame. Other drawings match exact object and layer names. Missing artwork leaves transparent frames. Ambiguous matches report an error. All PNGs use one crop and pivot. sheet.json records source ticks, trim offsets, timing and events.",
        result_fields: &[
            "output", "files", "metadata", "format", "scale", "clip_id", "clip_name",
            "reference_frame", "object_ids", "source_start", "source_end", "frame_count",
            "origin", "frame_size", "pivot", "fps", "looping",
        ],
    },
];

pub(super) fn command(name: &str) -> Option<&'static Spec> {
    COMMANDS.iter().find(|spec| spec.name == name)
}

pub(super) fn description() -> Value {
    let drawing = document_core::Document::for_new_project();
    let commands: Vec<_> = COMMANDS
        .iter()
        .map(|spec| {
            let positional: &[&str] = if spec.reads_project() {
                &["PROJECT"]
            } else {
                &[]
            };
            json!({
                "name": spec.name,
                "usage": spec.usage,
                "positional": positional,
                "options": spec.options,
                "required_options": spec.required,
                "summary": spec.result,
                "result_fields": spec.result_fields
            })
        })
        .collect();

    json!({
        "program": env!("CARGO_PKG_NAME"),
        "protocol_version": 1,
        "package_version": env!("CARGO_PKG_VERSION"),
        "commands": commands,
        "conventions": {
            "frame_base": 0,
            "coordinate_units": "artboard pixels; origin top left, +X right, +Y down",
            "object_scope": ["clip_id", "exposure_start", "object_id"],
            "color": "RGBA bytes in 0..255",
            "paths": "UTF-8; relative paths resolve from the working directory"
        },
        "response": {
            "success": {
                "version": 1,
                "ok": true,
                "revision": "sha256 of the input project bytes (project commands)",
                "result": "command-specific object"
            },
            "failure": {
                "version": 1,
                "ok": false,
                "error": {
                    "code": "machine-readable string",
                    "message": "actionable message",
                    "details": "optional object"
                }
            },
            "stream": "stdout",
            "success_exit": 0,
            "error_exit": 1
        },
        "options": {
            "clip": {
                "type": "unsigned integer",
                "meaning": "stable clip ID from inspect"
            },
            "set": {
                "type": "unsigned integer",
                "meaning": "stable animation set ID from inspect; exclusive with clip"
            },
            "frame": {
                "type": "unsigned integer",
                "meaning": "zero-based tick within the clip"
            },
            "from": {
                "type": "unsigned integer",
                "default": 0,
                "meaning": "first source tick to export, inclusive"
            },
            "to": {
                "type": "unsigned integer",
                "default": "last tick of the clip",
                "meaning": "last source tick to export, inclusive"
            },
            "object-id": {
                "type": "unsigned integer",
                "meaning": "stable ID within one drawing; exclusive with name"
            },
            "object-ids": {
                "type": "comma-separated positive integers",
                "meaning": "object IDs from the same clip and frame, including any member of a selected group"
            },
            "mode": {
                "type": "string",
                "enum": ["combined", "individual"],
                "default": "combined"
            },
            "name": {
                "type": "string",
                "meaning": "exact case-sensitive authored object name; exclusive with object-id"
            },
            "out": {
                "type": "path",
                "meaning": "New file for new, apply, preview or a single artboard or selection image. New directory for both still formats together, individual selection assets or animation exports."
            },
            "width": {
                "type": "integer",
                "min": 1,
                "max": document_core::MAX_DIMENSION,
                "default": drawing.width
            },
            "height": {
                "type": "integer",
                "min": 1,
                "max": document_core::MAX_DIMENSION,
                "default": drawing.height
            },
            "frames": {
                "type": "integer",
                "min": 1,
                "max": document_core::MAX_FRAMES,
                "default": drawing.frames
            },
            "fps": {
                "type": "integer",
                "min": 1,
                "max": document_core::MAX_FPS,
                "default": drawing.fps
            },
            "batch": {
                "type": "path",
                "meaning": "UTF-8 JSON file, or - to read stdin"
            },
            "scale": {
                "type": "number",
                "exclusive_min": 0,
                "meaning": "Positive decimal multiplier of native PNG resolution, for example 0.256.",
                "max": 32,
                "default": "1 for preview, export-artboard and export-selection. Animation exports use the project preset."
            },
            "format": {
                "type": "string",
                "enum": ["sheet", "frames", "sheet-and-frames", "packed", "png-sequence", "separate-sheets", "shared-sheet", "png", "svg", "both"],
                "meaning": "Clips and sets support named PNG sequences and individual or shared sprite sheets. Artboard, selection and batch support PNG, SVG or both. Animated selections support sheet, frames and sheet-and-frames.",
                "default": "sheet",
                "by_command": {
                    "export": {"enum": ["sheet", "frames", "sheet-and-frames", "packed", "png-sequence", "separate-sheets", "shared-sheet"], "default": "sheet", "with_set": {"enum": ["png-sequence", "separate-sheets", "shared-sheet", "frames", "packed"], "default": "shared-sheet"}},
                    "export-selection-animation": {"enum": ["sheet", "frames", "sheet-and-frames"], "default": "sheet"},
                    "export-artboard": {"enum": ["png", "svg", "both"], "default": "png", "requires": {"svg": {"scale":1}}},
                    "export-selection": {
                        "enum": ["png", "svg", "both"], "default": "png",
                        "requires": {"svg": {"scale": 1}}
                    }
                }
            },
            "layout": {
                "type": "string",
                "enum": ["grid", "packed"],
                "meaning": "Choose the layout of separate-sheets or shared-sheet. Uses the saved project preset when omitted."
            },
            "columns": {
                "type": "integer",
                "meaning": "grid sheets only; defaults to the project preset"
            }
        },
        "batch": {
            "version": 1,
            "revision": "copy revision from inspect or lookup",
            "operations": "ordered array of the operation objects below",
            "max_bytes": MAX_BATCH_BYTES
        },
        "operations": edit::describe(),
        "examples": {
            "lookup": [
                "lookup", "lantern-walk.vanim",
                "--name", "Sleeve fabric",
                "--clip", "1",
                "--frame", "0"
            ],
            "batch": {
                "version": 1,
                "revision": "<revision from lookup>",
                "operations": [
                    {
                        "op": "set_fill",
                        "clip_id": 1,
                        "exposure_start": 0,
                        "object_id": 20,
                        "fill": [220, 88, 54, 255]
                    },
                    {
                        "op": "set_position_key",
                        "clip_id": 1,
                        "exposure_start": 0,
                        "object_id": 20,
                        "frame": 0,
                        "position": [55, 60]
                    },
                    {
                        "op": "set_position_key",
                        "clip_id": 1,
                        "exposure_start": 0,
                        "object_id": 20,
                        "frame": 4,
                        "position": [63, 60]
                    }
                ]
            },
            "apply": [
                "apply", "lantern-walk.vanim",
                "--batch", "edits.json",
                "--out", "walk-edited.vanim"
            ],
            "preview": [
                "preview", "walk-edited.vanim",
                "--clip", "1",
                "--frame", "4",
                "--out", "pose.png"
            ],
            "export-selection": [
                "export-selection", "walk-edited.vanim",
                "--clip", "1",
                "--frame", "4",
                "--object-ids", "20",
                "--mode", "individual",
                "--out", "sprite-parts"
            ],
            "individual-svg": [
                "export-selection", "walk-edited.vanim",
                "--clip", "1", "--frame", "4", "--object-ids", "7,20",
                "--mode", "individual", "--format", "svg", "--out", "vector-parts"
            ],
            "export-selection-animation": [
                "export-selection-animation", "walk-edited.vanim",
                "--clip", "1", "--frame", "0", "--object-ids", "20",
                "--from", "0", "--to", "7", "--format", "sheet-and-frames",
                "--columns", "4", "--out", "sleeve-walk"
            ],
            "export": [
                "export", "walk-edited.vanim",
                "--clip", "1",
                "--out", "walk-sheet",
                "--format", "sheet",
                "--columns", "4"
            ]
        }
    })
}
