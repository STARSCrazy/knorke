//! Forbidden dependency edges (plan section 4, ADR 0001).
//!
//! Rules are checked against the *transitive* closure of normal and build dependencies of
//! each workspace crate, so a forbidden crate cannot sneak in through a helper. Dev
//! dependencies are ignored: tests may combine modules freely.

use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::process::ExitCode;

/// One rule: no crate matching `from` may depend (transitively) on a crate matching `to`
/// unless it also matches `except`. Patterns are exact names or a prefix ending in `*`.
struct Rule {
    from: &'static [&'static str],
    to: &'static [&'static str],
    except: &'static [&'static str],
    why: &'static str,
}

const DOMAIN_HEADLESS: &[&str] = &[
    "knorke-ai",
    "knorke-navigation",
    "knorke-perception",
    "knorke-grid",
];
const DEV_PLANE: &[&str] = &["knorke-agent", "knorke-cli", "knorke-mcp"];

const RULES: &[Rule] = &[
    Rule {
        from: DOMAIN_HEADLESS,
        to: &[
            "knorke-render",
            "knorke-lighting",
            "knorke-gpu",
            "knorke-light2d",
            "knorke-post",
            "knorke-runtime",
            "knorke-scene",
            "knorke-integration",
            "knorke-agent",
            "knorke-cli",
            "knorke-mcp",
            "sdl3*",
            "wgpu*",
        ],
        except: &[],
        why: "AI, navigation and perception run headless without graphics, runtime or agent plane",
    },
    // ADR 0014: the Chiaro port is split into knorke-gpu, knorke-post and knorke-light2d.
    Rule {
        from: &["knorke-light2d"],
        to: &[
            "knorke-post",
            "knorke-lighting",
            "knorke-render",
            "knorke-ai",
            "knorke-navigation",
            "knorke-perception",
            "knorke-grid",
            "knorke-runtime",
            "knorke-scene",
            "knorke-integration",
            "knorke-agent",
            "knorke-cli",
            "knorke-mcp",
            "sdl3*",
            "wgpu*",
        ],
        except: &[],
        why: "2D light (ADR 0014) knows neither post-processing, the CPU light or renderer, nor AI crates; GPU access only through knorke-gpu",
    },
    Rule {
        from: &["knorke-post"],
        to: &["knorke-light2d"],
        except: &[],
        why: "post-processing and 2D light are independent; only knorke-integration joins them (ADR 0014)",
    },
    Rule {
        from: &["knorke-lighting", "knorke-render"],
        to: &["knorke-light2d"],
        except: &[],
        why: "the frozen CPU light and the renderer do not know the GPU 2D light (ADR 0014)",
    },
    // The AI modules share only knorke-grid; knorke-integration joins them (ADR 0012).
    Rule {
        from: &["knorke-navigation"],
        to: &["knorke-perception", "knorke-ai"],
        except: &[],
        why: "navigation, perception and AI are independent; only knorke-integration joins them",
    },
    Rule {
        from: &["knorke-perception"],
        to: &["knorke-navigation", "knorke-ai"],
        except: &[],
        why: "navigation, perception and AI are independent; only knorke-integration joins them",
    },
    Rule {
        from: &["knorke-ai"],
        to: &["knorke-navigation", "knorke-perception"],
        except: &[],
        why: "navigation, perception and AI are independent; only knorke-integration joins them",
    },
    Rule {
        from: &["knorke-grid"],
        to: &["*"],
        except: &["knorke-types"],
        why: "knorke-grid holds the shared space types; closed dependency list (ADR 0012)",
    },
    Rule {
        from: &["knorke-render"],
        to: &["knorke-lighting"],
        except: &[],
        why: "renderer and lighting are independent; only knorke-integration joins them",
    },
    Rule {
        from: &["knorke-lighting"],
        to: &["knorke-render", "knorke-runtime", "knorke-scene"],
        except: &[],
        why: "lighting works on host-provided targets without the renderer or a world runtime",
    },
    // The GPU foundation serves 2D light, post and a later 3D renderer (ADR 0014).
    Rule {
        from: &["knorke-gpu"],
        to: &["knorke-*"],
        except: &["knorke-gpu", "knorke-types"],
        why: "knorke-gpu is the shared GPU foundation and knows no domain crate (ADR 0014)",
    },
    Rule {
        from: &["knorke-post"],
        to: &[
            "knorke-light2d",
            "knorke-lighting",
            "knorke-render",
            "knorke-ai",
            "knorke-navigation",
            "knorke-perception",
        ],
        except: &[],
        why: "post-processing math is shared by 2D and 3D output and stays free of lighting, rendering and AI",
    },
    Rule {
        from: &["knorke-runtime"],
        to: DEV_PLANE,
        except: &[],
        why: "a shipped game needs no agent plane",
    },
    Rule {
        from: &[
            "knorke-types",
            "knorke-math",
            "knorke-observe",
            "knorke-gpu",
            "knorke-render",
            "knorke-lighting",
            "knorke-light2d",
            "knorke-ai",
            "knorke-navigation",
            "knorke-perception",
            "knorke-grid",
        ],
        to: &["knorke-integration"],
        except: &[],
        why: "domain modules never import their adapters back",
    },
    // Foundation crates: every module depends on them, so their dependency trees are
    // closed lists (ADR 0006). Adding an entry needs an ADR change.
    Rule {
        from: &["knorke-types"],
        to: &["*"],
        except: &[],
        why: "knorke-types stays dependency-free so it cannot grow into a core",
    },
    Rule {
        from: &["knorke-math"],
        to: &["*"],
        except: &["bytemuck", "mint", "knorke-types", "libm"],
        why: "knorke-math may only use bytemuck (zero-copy casts), optional mint (interop), knorke-types (snapshots) and libm (deterministic math)",
    },
    Rule {
        from: &["knorke-observe"],
        to: &["*"],
        except: &["knorke-types"],
        why: "knorke-observe contracts must stay cheap to depend on for every module",
    },
    Rule {
        from: &["knorke-replay"],
        to: &["*"],
        except: &["knorke-types"],
        why: "replays may ship in release builds (flight recorder): knorke-types only",
    },
    Rule {
        from: &["knorke-exec"],
        to: &["*"],
        except: &[
            "knorke-types",
            "rayon",
            "rayon-core",
            "crossbeam-*",
            "either",
        ],
        why: "knorke-exec is the host's executor: knorke-types plus the rayon pool, nothing else",
    },
    Rule {
        from: &[
            "knorke-types",
            "knorke-math",
            "knorke-observe",
            "knorke-ai",
            "knorke-navigation",
            "knorke-perception",
            "knorke-grid",
            "knorke-render",
            "knorke-lighting",
            "knorke-gpu",
            "knorke-light2d",
        ],
        to: &["knorke-exec", "rayon*"],
        except: &[],
        why: "modules parallelise through the Parallel trait; the host picks the executor (ADR 0008)",
    },
];

fn matches(pattern: &str, name: &str) -> bool {
    match pattern.strip_suffix('*') {
        Some(prefix) => name.starts_with(prefix),
        None => pattern == name,
    }
}

fn matches_any(patterns: &[&str], name: &str) -> bool {
    patterns.iter().any(|p| matches(p, name))
}

pub(crate) fn metadata() -> Result<Value, String> {
    let out = std::process::Command::new(env!("CARGO"))
        .args([
            "metadata",
            "--format-version",
            "1",
            "--locked",
            "--all-features",
        ])
        .output()
        .map_err(|e| format!("cannot run cargo metadata: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "cargo metadata failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    serde_json::from_slice(&out.stdout).map_err(|e| format!("bad cargo metadata output: {e}"))
}

/// Names of workspace members that have a library target (xtask excluded).
pub(crate) fn workspace_libraries(meta: &Value) -> Vec<String> {
    let members: BTreeSet<&str> = meta["workspace_members"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    meta["packages"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|p| p["id"].as_str().is_some_and(|id| members.contains(id)))
        .filter(|p| {
            p["targets"].as_array().into_iter().flatten().any(|t| {
                t["kind"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|k| k.as_str() == Some("lib"))
            })
        })
        .filter_map(|p| p["name"].as_str().map(str::to_owned))
        .collect()
}

/// Package id → (name, ids of normal/build dependencies).
type Graph = BTreeMap<String, (String, Vec<String>)>;

fn graph(meta: &Value) -> Graph {
    let names: BTreeMap<&str, &str> = meta["packages"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| Some((p["id"].as_str()?, p["name"].as_str()?)))
        .collect();
    let mut g = Graph::new();
    for node in meta["resolve"]["nodes"].as_array().into_iter().flatten() {
        let Some(id) = node["id"].as_str() else {
            continue;
        };
        let deps = node["deps"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|d| {
                d["dep_kinds"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|k| k["kind"].as_str() != Some("dev"))
            })
            .filter_map(|d| d["pkg"].as_str().map(str::to_owned))
            .collect();
        g.insert(
            id.to_owned(),
            (names.get(id).copied().unwrap_or(id).to_owned(), deps),
        );
    }
    g
}

/// Returns the dependency chain from `start` to the first package matching `to`, if any.
fn forbidden_path(g: &Graph, start: &str, to: &[&str], except: &[&str]) -> Option<Vec<String>> {
    let mut parent: BTreeMap<&str, &str> = BTreeMap::new();
    let mut queue = VecDeque::from([start]);
    let mut seen = BTreeSet::from([start]);
    while let Some(id) = queue.pop_front() {
        let (name, deps) = g.get(id)?;
        if id != start && matches_any(to, name) && !matches_any(except, name) {
            let mut chain = vec![name.clone()];
            let mut cur = id;
            while let Some(&p) = parent.get(cur) {
                chain.push(g[p].0.clone());
                cur = p;
            }
            chain.reverse();
            return Some(chain);
        }
        for d in deps {
            if seen.insert(d) {
                parent.insert(d, id);
                queue.push_back(d);
            }
        }
    }
    None
}

fn violations(meta: &Value) -> Vec<String> {
    let g = graph(meta);
    let members: Vec<&str> = meta["workspace_members"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let mut out = Vec::new();
    for id in members {
        let Some((name, _)) = g.get(id) else { continue };
        for rule in RULES.iter().filter(|r| matches_any(r.from, name)) {
            if let Some(chain) = forbidden_path(&g, id, rule.to, rule.except) {
                out.push(format!("{}\n    rule: {}", chain.join(" -> "), rule.why));
            }
        }
    }
    out
}

pub(crate) fn run() -> ExitCode {
    let meta = match metadata() {
        Ok(m) => m,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    let v = violations(&meta);
    if v.is_empty() {
        eprintln!("dependency rules: ok ({} rules)", RULES.len());
        ExitCode::SUCCESS
    } else {
        eprintln!("forbidden dependency edges:");
        for line in &v {
            eprintln!("  {line}");
        }
        ExitCode::FAILURE
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn pkg(name: &str) -> Value {
        json!({ "id": format!("{name} 0.0.1"), "name": name, "targets": [{ "kind": ["lib"] }] })
    }

    fn node(name: &str, deps: &[(&str, &str)]) -> Value {
        let deps: Vec<Value> = deps
            .iter()
            .map(|(d, kind)| {
                let kind = if *kind == "normal" {
                    Value::Null
                } else {
                    json!(kind)
                };
                json!({ "pkg": format!("{d} 0.0.1"), "dep_kinds": [{ "kind": kind }] })
            })
            .collect();
        json!({ "id": format!("{name} 0.0.1"), "deps": deps })
    }

    fn meta(members: &[&str], nodes: Vec<Value>, all: &[&str]) -> Value {
        json!({
            "workspace_members": members.iter().map(|m| format!("{m} 0.0.1")).collect::<Vec<_>>(),
            "packages": all.iter().map(|n| pkg(n)).collect::<Vec<_>>(),
            "resolve": { "nodes": nodes },
        })
    }

    #[test]
    fn transitive_violation_is_reported_with_chain() {
        let m = meta(
            &["knorke-ai", "helper"],
            vec![
                node("knorke-ai", &[("helper", "normal")]),
                node("helper", &[("sdl3-sys", "normal")]),
                node("sdl3-sys", &[]),
            ],
            &["knorke-ai", "helper", "sdl3-sys"],
        );
        let v = violations(&m);
        assert_eq!(v.len(), 1);
        assert!(
            v[0].starts_with("knorke-ai -> helper -> sdl3-sys"),
            "{}",
            v[0]
        );
    }

    #[test]
    fn dev_dependencies_are_allowed() {
        let m = meta(
            &["knorke-render"],
            vec![
                node("knorke-render", &[("knorke-lighting", "dev")]),
                node("knorke-lighting", &[]),
            ],
            &["knorke-render", "knorke-lighting"],
        );
        assert!(violations(&m).is_empty());
    }

    #[test]
    fn types_must_stay_dependency_free() {
        let m = meta(
            &["knorke-types"],
            vec![
                node("knorke-types", &[("serde", "normal")]),
                node("serde", &[]),
            ],
            &["knorke-types", "serde"],
        );
        assert_eq!(violations(&m).len(), 1);
    }

    #[test]
    fn integration_may_join_modules() {
        let m = meta(
            &["knorke-integration"],
            vec![
                node(
                    "knorke-integration",
                    &[("knorke-render", "normal"), ("knorke-lighting", "normal")],
                ),
                node("knorke-render", &[]),
                node("knorke-lighting", &[]),
            ],
            &["knorke-integration", "knorke-render", "knorke-lighting"],
        );
        assert!(violations(&m).is_empty());
    }

    #[test]
    fn allowlisted_dependencies_pass_others_fail() {
        let m = meta(
            &["knorke-math"],
            vec![
                node("knorke-math", &[("bytemuck", "normal"), ("syn", "build")]),
                node("bytemuck", &[]),
                node("syn", &[]),
            ],
            &["knorke-math", "bytemuck", "syn"],
        );
        let v = violations(&m);
        assert_eq!(v.len(), 1);
        assert!(v[0].starts_with("knorke-math -> syn"), "{}", v[0]);
    }

    #[test]
    fn light2d_knows_no_post_light_or_ai_crates() {
        for forbidden in [
            "knorke-post",
            "knorke-lighting",
            "knorke-render",
            "knorke-ai",
        ] {
            let m = meta(
                &["knorke-light2d"],
                vec![
                    node("knorke-light2d", &[(forbidden, "normal")]),
                    node(forbidden, &[]),
                ],
                &["knorke-light2d", forbidden],
            );
            assert_eq!(violations(&m).len(), 1, "{forbidden}");
        }
        let ok = meta(
            &["knorke-light2d"],
            vec![
                node(
                    "knorke-light2d",
                    &[("knorke-types", "normal"), ("knorke-math", "normal")],
                ),
                node("knorke-types", &[]),
                node("knorke-math", &[]),
            ],
            &["knorke-light2d", "knorke-types", "knorke-math"],
        );
        assert!(violations(&ok).is_empty());
        let back = meta(
            &["knorke-lighting"],
            vec![
                node("knorke-lighting", &[("knorke-light2d", "normal")]),
                node("knorke-light2d", &[]),
            ],
            &["knorke-lighting", "knorke-light2d"],
        );
        assert_eq!(violations(&back).len(), 1);
    }

    #[test]
    fn ai_modules_stay_independent() {
        let m = meta(
            &["knorke-ai", "knorke-integration"],
            vec![
                node("knorke-ai", &[("knorke-navigation", "normal")]),
                node(
                    "knorke-integration",
                    &[("knorke-ai", "normal"), ("knorke-perception", "normal")],
                ),
                node("knorke-navigation", &[]),
                node("knorke-perception", &[]),
            ],
            &[
                "knorke-ai",
                "knorke-integration",
                "knorke-navigation",
                "knorke-perception",
            ],
        );
        let v = violations(&m);
        assert_eq!(v.len(), 1, "{v:?}");
        assert!(
            v[0].starts_with("knorke-ai -> knorke-navigation"),
            "{}",
            v[0]
        );
    }

    #[test]
    fn pattern_matching() {
        assert!(matches("sdl3*", "sdl3-sys"));
        assert!(matches("*", "anything"));
        assert!(!matches("knorke-ai", "knorke-ai-extra"));
    }
}
