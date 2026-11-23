//! `cargo xtask shaders`: builds shader packs (`.kpak`, ADR 0014 §3) from compiled shader
//! blobs and their shadercross reflection JSON.
//!
//! Input directory layout (what shadercross writes, e.g. Chiaro's `shaders/compiled`):
//! `<name>.<stage>.json` plus `<name>.<stage>.spv|msl|dxil` per program, stage `vert` or
//! `frag`. Output: `<out>/<pack>.<format>.kpak` for each requested format.
//!
//! The source checksum in the pack header is taken over the shader sources (every file of
//! `--sources`, default `<src>/../src` if it exists, sorted by name); without sources it is
//! taken over the inputs themselves. Compiling HLSL with shadercross is a later step.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use knorke_gpu::kpak::{
    ProgramSource, Reflection, ShaderFormat, Stage, source_checksum, write_pack,
};

const USAGE: &str = "usage: cargo xtask shaders --src <compiled dir> [--name <pack>] [--out <dir>]\n\
     \x20                           [--sources <hlsl dir>] [--only a.frag,b.vert] [--formats spirv,msl,dxil]\n\n\
     defaults: --name knorke-chiaro, --out target/shaders, --sources <src>/../src, all programs, all formats";

struct Args {
    src: PathBuf,
    name: String,
    out: PathBuf,
    sources: Option<PathBuf>,
    only: Option<Vec<String>>,
    formats: Vec<ShaderFormat>,
}

fn parse_args(args: &[String]) -> Result<Args, String> {
    let mut src = None;
    let mut name = "knorke-chiaro".to_owned();
    let mut out = PathBuf::from("target/shaders");
    let mut sources = None;
    let mut only = None;
    let mut formats = ShaderFormat::ALL.to_vec();
    let mut it = args.iter();
    while let Some(flag) = it.next() {
        let mut value = || it.next().cloned().ok_or(format!("{flag} needs a value"));
        match flag.as_str() {
            "--src" => src = Some(PathBuf::from(value()?)),
            "--name" => name = value()?,
            "--out" => out = PathBuf::from(value()?),
            "--sources" => sources = Some(PathBuf::from(value()?)),
            "--only" => only = Some(value()?.split(',').map(str::to_owned).collect()),
            "--formats" => {
                formats = value()?
                    .split(',')
                    .map(|t| {
                        ShaderFormat::ALL
                            .into_iter()
                            .find(|f| f.tag() == t)
                            .ok_or(format!("unknown format '{t}'"))
                    })
                    .collect::<Result<_, _>>()?;
            }
            other => return Err(format!("unknown argument '{other}'")),
        }
    }
    Ok(Args {
        src: src.ok_or("--src is required")?,
        name,
        out,
        sources,
        only,
        formats,
    })
}

/// Reads the four counts SDL needs from a shadercross reflection file.
fn reflection(path: &Path) -> Result<Reflection, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let json: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    let field = |key: &str| -> Result<u16, String> {
        json.get(key)
            .and_then(serde_json::Value::as_u64)
            .and_then(|v| u16::try_from(v).ok())
            .ok_or(format!("{}: missing or invalid '{key}'", path.display()))
    };
    Ok(Reflection {
        samplers: field("samplers")?,
        storage_textures: field("storage_textures")?,
        storage_buffers: field("storage_buffers")?,
        uniform_buffers: field("uniform_buffers")?,
    })
}

fn sorted_files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file())
        .collect();
    files.sort();
    Ok(files)
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn checksum_of(files: &[PathBuf]) -> Result<u64, String> {
    let data: Vec<(String, Vec<u8>)> = files
        .iter()
        .map(|p| {
            std::fs::read(p)
                .map(|b| (file_name(p), b))
                .map_err(|e| format!("{}: {e}", p.display()))
        })
        .collect::<Result<_, _>>()?;
    Ok(source_checksum(
        data.iter().map(|(n, b)| (n.as_str(), b.as_slice())),
    ))
}

fn build(args: &Args) -> Result<(), String> {
    let mut programs: Vec<(String, Stage, Reflection)> = Vec::new();
    let mut inputs = Vec::new();
    for path in sorted_files(&args.src)? {
        let fname = file_name(&path);
        let Some(name) = fname.strip_suffix(".json") else {
            continue;
        };
        if args
            .only
            .as_ref()
            .is_some_and(|o| !o.iter().any(|n| n == name))
        {
            continue;
        }
        let stage = if name.ends_with(".vert") {
            Stage::Vertex
        } else if name.ends_with(".frag") {
            Stage::Fragment
        } else {
            return Err(format!(
                "{fname}: unsupported stage (expected .vert or .frag)"
            ));
        };
        programs.push((name.to_owned(), stage, reflection(&path)?));
        inputs.push(path.clone());
        for f in ShaderFormat::ALL {
            let blob = args.src.join(format!("{name}.{}", f.blob_extension()));
            if blob.is_file() {
                inputs.push(blob);
            }
        }
    }
    if let Some(only) = &args.only
        && let Some(missing) = only.iter().find(|n| !programs.iter().any(|p| &p.0 == *n))
    {
        return Err(format!(
            "--only: no program '{missing}' in {}",
            args.src.display()
        ));
    }
    if programs.is_empty() {
        return Err(format!(
            "no reflection files (*.json) in {}",
            args.src.display()
        ));
    }

    let sources = args.sources.clone().or_else(|| {
        args.src
            .parent()
            .map(|p| p.join("src"))
            .filter(|p| p.is_dir())
    });
    let checksum = match &sources {
        Some(dir) => checksum_of(&sorted_files(dir)?)?,
        None => checksum_of(&inputs)?,
    };

    std::fs::create_dir_all(&args.out).map_err(|e| format!("{}: {e}", args.out.display()))?;
    for &format in &args.formats {
        let list = programs
            .iter()
            .map(|(name, stage, reflection)| {
                let blob = args.src.join(format!("{name}.{}", format.blob_extension()));
                let code = std::fs::read(&blob).map_err(|e| format!("{}: {e}", blob.display()))?;
                Ok(ProgramSource {
                    name: name.clone(),
                    stage: *stage,
                    entry_point: format.default_entry_point().to_owned(),
                    reflection: *reflection,
                    code,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let bytes = write_pack(format, checksum, &list).map_err(|d| d.message)?;
        let path = args.out.join(format.pack_file_name(&args.name));
        std::fs::write(&path, &bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        eprintln!(
            "wrote {} ({} programs, {} bytes, sources {checksum:#018x})",
            path.display(),
            list.len(),
            bytes.len()
        );
    }
    Ok(())
}

/// Entry point of `cargo xtask shaders`.
pub(crate) fn run(args: &[String]) -> ExitCode {
    let result = parse_args(args).and_then(|a| build(&a));
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}\n\n{USAGE}");
            ExitCode::FAILURE
        }
    }
}
