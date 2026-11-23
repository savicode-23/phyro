use crate::{edit, inspect, output, CliError};
use document_core::AnimationProject;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

mod discovery;
pub use discovery::HELP;

pub fn is_help(args: &[OsString]) -> bool {
    let help = |arg: &OsString| arg == "--help" || arg == "-h" || arg == "help";
    args.is_empty()
        || matches!(args, [a] if help(a))
        || matches!(args, [a,b] if help(b) && a.to_str().is_some_and(|name| discovery::command(name).is_some()))
}

struct Options(BTreeMap<String, String>);

impl Options {
    fn get(&self, name: &str) -> Option<&str> {
        self.0.get(name).map(String::as_str)
    }

    fn required(&self, name: &str) -> Result<&str, CliError> {
        self.get(name)
            .ok_or_else(|| usage(format!("Supply --{name} with a value.")))
    }

    fn number<T: std::str::FromStr>(&self, name: &str) -> Result<Option<T>, CliError> {
        self.get(name)
            .map(|value| {
                value
                    .parse()
                    .map_err(|_| usage(format!("--{name} must be an unsigned integer in range.")))
            })
            .transpose()
    }

    fn object_ids(&self) -> Result<Vec<u64>, CliError> {
        self.required("object-ids")?
            .split(',')
            .map(|part| {
                part.trim().parse::<u64>().ok().filter(|&id| id > 0).ok_or_else(|| {
                    usage("Give --object-ids a comma-separated list of positive IDs from the chosen drawing.")
                })
            })
            .collect()
    }

    fn scale(&self) -> Result<Option<(u32, u32)>, CliError> {
        let Some(value) = self.get("scale") else {
            return Ok(None);
        };
        let invalid = || {
            CliError::new(
                "invalid_argument",
                "Use a positive decimal PNG scale up to 32, for example 0.256, 1 or 2.",
            )
        };
        let (whole, decimals) = value.split_once('.').unwrap_or((value, ""));
        if !whole
            .bytes()
            .chain(decimals.bytes())
            .all(|c| c.is_ascii_digit())
        {
            return Err(invalid());
        }
        let decimals = decimals.trim_end_matches('0');
        let numerator = format!("{whole}{decimals}")
            .parse::<u64>()
            .map_err(|_| invalid())?;
        let denominator = 10_u64
            .checked_pow(decimals.len().try_into().map_err(|_| invalid())?)
            .ok_or_else(invalid)?;
        if numerator == 0 || u128::from(numerator) > u128::from(denominator) * 32 {
            return Err(invalid());
        }
        let (mut a, mut b) = (numerator, denominator);
        while b != 0 {
            (a, b) = (b, a % b);
        }
        Ok(Some((
            (numerator / a).try_into().map_err(|_| invalid())?,
            (denominator / a).try_into().map_err(|_| invalid())?,
        )))
    }
}

fn usage(message: impl Into<String>) -> CliError {
    CliError::new("invalid_arguments", message)
}

pub fn run(args: Vec<OsString>) -> Result<Value, CliError> {
    let args: Vec<String> = args
        .into_iter()
        .map(|arg| {
            arg.into_string()
                .map_err(|_| usage("Supply UTF-8 text and paths."))
        })
        .collect::<Result<_, _>>()?;
    let mut args = args.into_iter();
    let name = args.next().unwrap_or_default();
    let spec = discovery::command(&name)
        .ok_or_else(|| usage(format!("Unknown command '{name}'. Run phyro-cli --help.")))?;
    if name == "describe" {
        if args.next().is_some() {
            return Err(usage(spec.usage));
        }
        return Ok(json!({"version":1,"ok":true,"result":discovery::description()}));
    }
    let project_path = if spec.reads_project() {
        Some(args.next().ok_or_else(|| usage(spec.usage))?)
    } else {
        None
    };
    let mut options = BTreeMap::new();
    while let Some(argument) = args.next() {
        let flag = argument
            .strip_prefix("--")
            .ok_or_else(|| usage(spec.usage))?;
        if !spec.options.contains(&flag) {
            return Err(usage(format!("Unknown option {argument}. {}", spec.usage)));
        }
        let value = args
            .next()
            .ok_or_else(|| usage(format!("Supply a value for {argument}.")))?;
        if options.insert(flag.to_string(), value).is_some() {
            return Err(usage(format!("Supply {argument} only once.")));
        }
    }
    let options = Options(options);
    for key in spec.required {
        options.required(key)?;
    }
    let Some(project_path) = project_path else {
        return Ok(json!({"version":1,"ok":true,"result":new_project(&options)?}));
    };
    let clip = options.number::<u64>("clip")?;
    let frame = options.number::<u32>("frame")?;
    let bytes = read_limited(
        File::open(&project_path).map_err(|e| {
            CliError::new(
                "read_project",
                format!("Could not open '{project_path}': {e}"),
            )
        })?,
        project_io::MAX_ARCHIVE_BYTES,
    )
    .map_err(|e| CliError::new("read_project", e.to_string()))?;
    let revision = format!("sha256:{:x}", Sha256::digest(&bytes));
    let mut project = project_io::load_animation_bytes(&bytes)
        .map_err(|e| CliError::new("invalid_project", e.to_string()))?;
    drop(bytes);
    let mut still_scale = 1.;
    if let Some((pixels, units)) = options.scale()? {
        project.preset.set_raster_ratio(pixels, units);
        still_scale = project.preset.raster_scale();
    }
    // Required options were checked against the advertised contract above.
    let required_clip = || clip.ok_or_else(|| usage("Supply --clip with a clip ID from inspect."));
    let required_frame = || frame.ok_or_else(|| usage("Supply --frame with a zero-based tick."));
    let result = match name.as_str() {
        "inspect" => inspect::inspect(&project, clip, frame)?,
        "lookup" => inspect::lookup(
            &project,
            options.get("name"),
            options.number("object-id")?,
            clip,
            frame,
        )?,
        "preview" => output::preview(
            &project,
            required_clip()?,
            required_frame()?,
            Path::new(options.required("out")?),
            still_scale,
        )?,
        "export" => {
            let target = match (clip, options.number("set")?) {
                (Some(id), None) => project_io::AnimationExportTarget::Clip(id),
                (None, Some(id)) => project_io::AnimationExportTarget::Set(id),
                _ => return Err(usage("Choose exactly one --clip or --set ID from inspect.")),
            };
            output::export_project(
                &project,
                target,
                Path::new(options.required("out")?),
                options.get("format"),
                options.number("columns")?,
                options.get("layout"),
            )?
        }
        "export-artboard" | "export-selection" => {
            let ids;
            let target = if name == "export-artboard" {
                project_io::StillTarget::Artboard
            } else {
                ids = options.object_ids()?;
                match options.get("mode").unwrap_or("combined") {
                    "combined" => project_io::StillTarget::Selection(&ids),
                    "individual" => project_io::StillTarget::Objects(&ids),
                    _ => return Err(usage("Choose combined or individual for --mode.")),
                }
            };
            output::export_current_frame(
                inspect::clip_by_id(&project, required_clip()?)?,
                required_frame()?,
                target,
                options.get("format").unwrap_or("png"),
                Path::new(options.required("out")?),
                still_scale,
            )?
        }
        "export-selection-animation" => {
            let clip = inspect::clip_by_id(&project, required_clip()?)?;
            let range = project_io::SelectionRange {
                reference: required_frame()?,
                first: options.number("from")?.unwrap_or(0),
                last: options.number("to")?.unwrap_or(clip.frames - 1),
            };
            let settings = output::sprite_settings(
                project.preset,
                options.get("format").unwrap_or("sheet"),
                options.number("columns")?,
            )?;
            output::selection_animation(
                clip,
                &options.object_ids()?,
                range,
                Path::new(options.required("out")?),
                settings,
            )?
        }
        "apply" => apply(
            &mut project,
            &revision,
            options.required("batch")?,
            options.required("out")?,
        )?,
        _ => unreachable!("every advertised command has a handler"),
    };
    Ok(json!({"version":1,"ok":true,"revision":revision,"result":result}))
}

fn read_limited(reader: impl Read, max: u64) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader.take(max + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Input exceeds {max} bytes."),
        ));
    }
    Ok(bytes)
}

const MAX_BATCH_BYTES: u64 = 16 * 1024 * 1024;

fn new_project(options: &Options) -> Result<Value, CliError> {
    let default = document_core::Document::for_new_project();
    let doc = document_core::Document {
        width: options.number("width")?.unwrap_or(default.width),
        height: options.number("height")?.unwrap_or(default.height),
        fps: options.number("fps")?.unwrap_or(default.fps),
        frames: options.number("frames")?.unwrap_or(default.frames),
        ..default
    };
    doc.validate().map_err(usage)?;
    let project = AnimationProject::from_document(doc);
    let path = output::new_destination(Path::new(options.required("out")?))?;
    save_new(&project, &path)?;
    let clip = &project.clips[0];
    Ok(
        json!({"output":path,"clip_id":clip.id,"exposure_start":0,"layer_id":clip.drawing_at(0).layers[0].id,
        "frames":clip.frames,"fps":clip.fps,"artboard":{"width":clip.drawing_at(0).width,"height":clip.drawing_at(0).height}}),
    )
}

fn save_new(project: &AnimationProject, path: &Path) -> Result<(), CliError> {
    project_io::save_animation(path, project, false).map_err(|e| {
        let code = if e.kind() == io::ErrorKind::AlreadyExists {
            "output_exists"
        } else {
            "save_project"
        };
        CliError::new(
            code,
            format!("Could not save a new project at '{}': {e}", path.display()),
        )
        .with_details(json!({"output":path}))
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Batch {
    version: u32,
    revision: String,
    operations: Value,
}

fn apply(
    project: &mut AnimationProject,
    revision: &str,
    batch: &str,
    out: &str,
) -> Result<Value, CliError> {
    let bytes = if batch == "-" {
        read_limited(io::stdin().lock(), MAX_BATCH_BYTES)
    } else {
        File::open(batch).and_then(|file| read_limited(file, MAX_BATCH_BYTES))
    }
    .map_err(|e| CliError::new("read_batch", format!("Could not read batch '{batch}': {e}")))?;
    let batch: Batch =
        serde_json::from_slice(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes))
            .map_err(|e| CliError::new("invalid_batch", format!("Invalid batch JSON: {e}")))?;
    if batch.version != 1 {
        return Err(CliError::new("unsupported_version", "Use batch version 1."));
    }
    if batch.revision != revision {
        return Err(CliError::new(
            "stale_revision",
            "The project revision differs. Inspect it again before editing.",
        )
        .with_details(json!({"requested":batch.revision,"actual":revision})));
    }
    let path = output::new_destination(Path::new(out))?;
    let operations = edit::apply(project, &batch.operations)?;
    save_new(project, &path)?;
    Ok(json!({"output":path,"operations":operations}))
}
