//! `cargo xtask shaders`: builds shader packs (`.kpak`, ADR 0014 §3) from compiled shader
//! blobs and their reflection JSON, and compiles Knorke's own HLSL with DXC.
//!
//! Input directory layout (what shadercross writes): `<name>.<stage>.json` plus
//! `<name>.<stage>.spv|msl|dxil` per program, stage `vert` or `frag`. Several `--src`
//! directories go into one pack; without `--src` they are the repo's two shader dirs
//! ([`DEFAULT_SRC`]), so plain `cargo xtask shaders` builds the full pack. Output:
//! `<out>/<pack>.<format>.kpak` for each requested format; a program without a blob of a
//! format is left out of that format's pack (only Chiaro's shadercross blobs have MSL).
//!
//! The source checksum in the pack header is taken over the shader sources (every file of
//! `--sources`, default `<src>/../src` and `<src>/../shared` of each `--src` that has them,
//! sorted by name); without sources it is taken over the inputs themselves.
//!
//! `--compile <dir>` compiles every `<dir>/src/<name>.<vert|frag>.hlsl` into
//! `<dir>/compiled/<name>.<stage>.spv` and `.dxil` with DXC (`--dxc <exe>`, else the
//! environment variable `KNORKE_DXC`, else `dxc` on the PATH; it must be a build with SPIR-V
//! support, the one of the Windows SDK has none), with `<dir>/shared` (the includes shared with
//! Chiaro's programs in `<dir>/chiaro/src`) on the include path. The reflection JSON (four
//! counts) is written by hand next to the blobs and checked by the tests of the crate that owns
//! the shaders; `compiled/sources.sum` records the checksum of the sources the blobs were built
//! from (`src` and `shared`); the pack is built afterwards as above. An `.msl` blob of a
//! recompiled program is deleted: DXC cannot renew it, and a stale one would go into the MSL
//! pack.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use knorke_gpu::kpak::{
    ProgramSource, Reflection, ShaderFormat, Stage, source_checksum, write_pack,
};

const USAGE: &str = "usage: cargo xtask shaders --src <compiled dir> [--src <more>] [--name <pack>] [--out <dir>]\n\
     \x20                           [--sources <hlsl dir>] [--only a.frag,b.vert] [--formats spirv,msl,dxil]\n\
     \x20      cargo xtask shaders --compile <dir with src/> [--dxc <dxc.exe>]\n\n\
     defaults: --src the repo's shader dirs, --name knorke-chiaro,\n\
     \x20         --out target/shaders, --sources <src>/../src, all programs, all formats";

/// The repo's shader dirs, relative to the workspace root: Chiaro's programs that Knorke still
/// uses (shadercross blobs of chiaro 9bc5b9b) and Knorke's own (DXC).
const DEFAULT_SRC: [&str; 2] = [
    "crates/knorke-integration/shaders/chiaro/compiled",
    "crates/knorke-integration/shaders/compiled",
];

struct Args {
    src: Vec<PathBuf>,
    compile: Option<PathBuf>,
    dxc: Option<PathBuf>,
    name: String,
    out: PathBuf,
    sources: Option<PathBuf>,
    only: Option<Vec<String>>,
    formats: Vec<ShaderFormat>,
}

fn parse_args(args: &[String]) -> Result<Args, String> {
    let mut src = Vec::new();
    let mut compile = None;
    let mut dxc = None;
    let mut name = "knorke-chiaro".to_owned();
    let mut out = PathBuf::from("target/shaders");
    let mut sources = None;
    let mut only = None;
    let mut formats = ShaderFormat::ALL.to_vec();
    let mut it = args.iter();
    while let Some(flag) = it.next() {
        let mut value = || it.next().cloned().ok_or(format!("{flag} needs a value"));
        match flag.as_str() {
            "--src" => src.push(PathBuf::from(value()?)),
            "--compile" => compile = Some(PathBuf::from(value()?)),
            "--dxc" => dxc = Some(PathBuf::from(value()?)),
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
    if src.is_empty() {
        src = DEFAULT_SRC.iter().map(PathBuf::from).collect();
    }
    Ok(Args {
        src,
        compile,
        dxc,
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
    // Name, stage, reflection and the directory of the blobs.
    let mut programs: Vec<(String, Stage, Reflection, &Path)> = Vec::new();
    let mut inputs = Vec::new();
    let mut files = Vec::new();
    for dir in &args.src {
        files.extend(sorted_files(dir)?.into_iter().map(|f| (dir.as_path(), f)));
    }
    for (dir, path) in files {
        let fname = file_name(&path);
        let Some(name) = fname.strip_suffix(".json") else {
            continue; // blobs, sources.sum
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
        programs.push((name.to_owned(), stage, reflection(&path)?, dir));
        inputs.push(path.clone());
        for f in ShaderFormat::ALL {
            let blob = dir.join(format!("{name}.{}", f.blob_extension()));
            if blob.is_file() {
                inputs.push(blob);
            }
        }
    }
    if let Some(only) = &args.only
        && let Some(missing) = only.iter().find(|n| !programs.iter().any(|p| &p.0 == *n))
    {
        return Err(format!("--only: no program '{missing}'"));
    }
    if programs.is_empty() {
        return Err("no reflection files (*.json) in the --src directories".to_owned());
    }

    let sources: Vec<PathBuf> = match &args.sources {
        Some(dir) => vec![dir.clone()],
        None => args
            .src
            .iter()
            .filter_map(|d| d.parent())
            .flat_map(|p| [p.join("src"), p.join("shared")])
            .filter(|p| p.is_dir())
            .collect(),
    };
    let checksum = if sources.is_empty() {
        checksum_of(&inputs)?
    } else {
        let mut files = Vec::new();
        for dir in &sources {
            files.extend(sorted_files(dir)?);
        }
        files.sort_by_key(|p| file_name(p));
        checksum_of(&files)?
    };

    std::fs::create_dir_all(&args.out).map_err(|e| format!("{}: {e}", args.out.display()))?;
    for &format in &args.formats {
        let list = programs
            .iter()
            .filter(|(name, _, _, dir)| {
                dir.join(format!("{name}.{}", format.blob_extension()))
                    .is_file()
            })
            .map(|(name, stage, reflection, dir)| {
                let blob = dir.join(format!("{name}.{}", format.blob_extension()));
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
            "wrote {} ({} of {} programs, {} bytes, sources {checksum:#018x})",
            path.display(),
            list.len(),
            programs.len(),
            bytes.len()
        );
    }
    Ok(())
}

/// Compiles `<dir>/src/*.{vert,frag}.hlsl` into `<dir>/compiled` as SPIR-V and DXIL.
fn compile(dir: &Path, dxc: Option<&Path>) -> Result<(), String> {
    let dxc = dxc
        .map(Path::to_path_buf)
        .or_else(|| std::env::var_os("KNORKE_DXC").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("dxc"));
    let out = dir.join("compiled");
    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    let shared = dir.join("shared");
    let mut sources = sorted_files(&dir.join("src"))?;
    if shared.is_dir() {
        sources.extend(sorted_files(&shared)?);
        sources.sort_by_key(|p| file_name(p));
    }
    let mut compiled = 0;
    for path in sorted_files(&dir.join("src"))? {
        let fname = file_name(&path);
        let Some(name) = fname.strip_suffix(".hlsl") else {
            continue; // includes (.hlsli)
        };
        let profile = if name.ends_with(".vert") {
            "vs_6_0"
        } else if name.ends_with(".frag") {
            "ps_6_0"
        } else {
            return Err(format!(
                "{fname}: expected <name>.vert.hlsl or <name>.frag.hlsl"
            ));
        };
        // `-spirv` alone is what shadercross passes: Chiaro's blobs come out byte for byte.
        for (ext, extra) in [("spv", Some("-spirv")), ("dxil", None)] {
            let target = out.join(format!("{name}.{ext}"));
            let status = std::process::Command::new(&dxc)
                .args(["-T", profile, "-E", "main"])
                .args(extra)
                .arg("-I")
                .arg(&shared)
                .arg(&path)
                .arg("-Fo")
                .arg(&target)
                .status()
                .map_err(|e| format!("running {}: {e}", dxc.display()))?;
            if !status.success() {
                return Err(format!("{}: dxc failed for {ext}", path.display()));
            }
        }
        // Stale once the SPIR-V is new; missing is fine (the program leaves the MSL pack).
        let _ = std::fs::remove_file(out.join(format!("{name}.msl")));
        if !out.join(format!("{name}.json")).is_file() {
            return Err(format!("{name}.json is missing in {}", out.display()));
        }
        compiled += 1;
    }
    // What the blobs were built from, for the owning crate's test that they are up to date.
    let sum = checksum_of(&sources)?;
    let sum_file = out.join("sources.sum");
    std::fs::write(&sum_file, format!("{sum:016x}\n"))
        .map_err(|e| format!("{}: {e}", sum_file.display()))?;
    eprintln!("compiled {compiled} shaders into {}", out.display());
    Ok(())
}

/// Entry point of `cargo xtask shaders`.
pub(crate) fn run(args: &[String]) -> ExitCode {
    let result = parse_args(args).and_then(|a| {
        if let Some(dir) = &a.compile {
            compile(dir, a.dxc.as_deref())?;
        }
        build(&a)
    });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}\n\n{USAGE}");
            ExitCode::FAILURE
        }
    }
}
