//! Authoring state (plan section 8.2, ADR 0018): what a scene *is*, as text.
//!
//! - A [`Scene`] is entities, each a set of components, each component a JSON value under
//!   its versioned id (`knorke.stealth.guard/1`). It carries a revision.
//! - A [`Registry`] knows the component types: Rust types that implement [`Component`].
//!   The type is the one schema source: it reads the value, checks it and describes itself
//!   as JSON Schema.
//! - A scene changes only through a [`ChangeSet`]: applied as a whole against the revision
//!   it was written for, or not at all, and answered with the change set that undoes it.
//!
//! Running a scene is not this crate's business: a profile in `knorke-integration` builds
//! the runtime state from it (authoring is not runtime).

use std::collections::BTreeMap;

use knorke_types::{Diagnostic, DiagnosticCode, Severity};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// The format this crate reads and writes.
pub const SCHEMA: &str = "knorke.scene/1";

/// The file is not JSON of the scene's shape.
pub const MALFORMED: DiagnosticCode = DiagnosticCode::new("knorke.scene.malformed");
/// The file names another format than [`SCHEMA`].
pub const UNKNOWN_SCHEMA: DiagnosticCode = DiagnosticCode::new("knorke.scene.unknown_schema");
/// A component id no registered type answers to.
pub const UNKNOWN_COMPONENT: DiagnosticCode = DiagnosticCode::new("knorke.scene.unknown_component");
/// A component value its type cannot read.
pub const INVALID_COMPONENT: DiagnosticCode = DiagnosticCode::new("knorke.scene.invalid_component");
/// A change set written for another revision.
pub const REVISION_MISMATCH: DiagnosticCode = DiagnosticCode::new("knorke.scene.revision_mismatch");
/// A change names an entity the scene does not have.
pub const UNKNOWN_ENTITY: DiagnosticCode = DiagnosticCode::new("knorke.scene.unknown_entity");
/// A change creates an entity the scene already has.
pub const DUPLICATE_ENTITY: DiagnosticCode = DiagnosticCode::new("knorke.scene.duplicate_entity");
/// A change removes a component the entity does not have.
pub const MISSING_COMPONENT: DiagnosticCode = DiagnosticCode::new("knorke.scene.missing_component");

/// An entity: its components by id.
pub type Entity = BTreeMap<String, Value>;

/// A scene as stored. `BTreeMap`s keep the text stable, so a diff shows what changed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Scene {
    /// The format, [`SCHEMA`].
    pub schema: String,
    /// Counts the applied change sets.
    pub revision: u64,
    /// The entities by name.
    pub entities: BTreeMap<String, Entity>,
}

impl Default for Scene {
    fn default() -> Self {
        Self {
            schema: SCHEMA.into(),
            revision: 0,
            entities: BTreeMap::new(),
        }
    }
}

/// One step of a [`ChangeSet`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind")]
pub enum Change {
    /// Adds an entity without components.
    #[serde(rename = "entity.create")]
    EntityCreate {
        /// Its name.
        entity: String,
    },
    /// Removes an entity and its components.
    #[serde(rename = "entity.remove")]
    EntityRemove {
        /// Its name.
        entity: String,
    },
    /// Sets or replaces one component of an entity.
    #[serde(rename = "component.set")]
    ComponentSet {
        /// The entity.
        entity: String,
        /// The component id.
        component: String,
        /// The whole new value.
        value: Value,
    },
    /// Removes one component of an entity.
    #[serde(rename = "component.remove")]
    ComponentRemove {
        /// The entity.
        entity: String,
        /// The component id.
        component: String,
    },
}

/// Changes that apply together or not at all.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChangeSet {
    /// The revision the changes were written for.
    pub base_revision: u64,
    /// The steps, in order.
    pub changes: Vec<Change>,
}

/// A component type. The Rust type is the schema: `serde` reads and writes it, `schemars`
/// describes it, [`check`](Self::check) holds what the type system cannot say.
pub trait Component: Serialize + DeserializeOwned + JsonSchema {
    /// Versioned id, e.g. `knorke.stealth.guard/1`. A change that old scenes cannot be read
    /// with gets a new number.
    const ID: &'static str;

    /// Reports values that are well-typed but wrong (a radius of zero), each
    /// [`at`](Diagnostic::at) `path` plus `/<field>`.
    fn check(&self, path: &str, out: &mut Vec<Diagnostic>) {
        let _ = (path, out);
    }
}

/// The path of a component for diagnostics.
pub fn component_path(entity: &str, component: &str) -> String {
    format!("entities/{entity}/components/{component}")
}

struct Kind {
    schema: fn() -> Value,
    check: fn(&Value, &str, &mut Vec<Diagnostic>),
}

fn schema_of<C: Component>() -> Value {
    serde_json::to_value(schemars::schema_for!(C)).unwrap_or(Value::Null)
}

fn check_as<C: Component>(value: &Value, path: &str, out: &mut Vec<Diagnostic>) {
    match C::deserialize(value) {
        Ok(c) => c.check(path, out),
        Err(e) => out.push(Diagnostic::error(INVALID_COMPONENT, e.to_string()).at(path)),
    }
}

/// A rule over the whole scene (references, "exactly one of").
pub type Rule = fn(&Scene, &mut Vec<Diagnostic>);

/// The component types and scene rules a host works with.
#[derive(Default)]
pub struct Registry {
    kinds: BTreeMap<&'static str, Kind>,
    rules: Vec<Rule>,
}

impl Registry {
    /// Makes component type `C` known.
    pub fn register<C: Component>(&mut self) -> &mut Self {
        self.kinds.insert(
            C::ID,
            Kind {
                schema: schema_of::<C>,
                check: check_as::<C>,
            },
        );
        self
    }

    /// Adds a rule over the whole scene. Rules run after every component was checked.
    pub fn rule(&mut self, rule: Rule) -> &mut Self {
        self.rules.push(rule);
        self
    }

    /// The JSON Schema of every component type, by id.
    pub fn schemas(&self) -> BTreeMap<&'static str, Value> {
        self.kinds
            .iter()
            .map(|(&id, kind)| (id, (kind.schema)()))
            .collect()
    }

    /// Everything wrong with `scene`. An empty answer, or warnings only, is a valid scene.
    pub fn validate(&self, scene: &Scene) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        if scene.schema != SCHEMA {
            let message = format!("expected schema {SCHEMA}, found {}", scene.schema);
            out.push(Diagnostic::error(UNKNOWN_SCHEMA, message).at("schema"));
        }
        for (name, entity) in &scene.entities {
            for (id, value) in entity {
                let path = component_path(name, id);
                match self.kinds.get(id.as_str()) {
                    Some(kind) => (kind.check)(value, &path, &mut out),
                    None => out.push(
                        Diagnostic::error(UNKNOWN_COMPONENT, "no such component type").at(path),
                    ),
                }
            }
        }
        for rule in &self.rules {
            rule(scene, &mut out);
        }
        out
    }
}

/// `true` if `diagnostics` holds an error.
pub fn has_errors(diagnostics: &[Diagnostic]) -> bool {
    diagnostics.iter().any(|d| d.severity == Severity::Error)
}

/// A diagnostic as JSON, for tools: `severity`, `code`, `message`, `path`.
pub fn diagnostic_json(d: &Diagnostic) -> Value {
    json!({ "severity": d.severity.as_str(), "code": d.code.as_str(), "message": d.message, "path": d.path })
}

impl Scene {
    /// Reads a scene from its text.
    ///
    /// # Errors
    /// [`MALFORMED`] with the parser's message.
    pub fn from_text(text: &str) -> Result<Self, Diagnostic> {
        serde_json::from_str(text).map_err(|e| Diagnostic::error(MALFORMED, e.to_string()))
    }

    /// The scene as text: indented JSON with a final newline.
    pub fn to_text(&self) -> String {
        let mut text = serde_json::to_string_pretty(self).unwrap_or_default();
        text.push('\n');
        text
    }

    /// Component `C` of `entity`, if it has one that reads.
    pub fn get<C: Component>(&self, entity: &str) -> Option<C> {
        C::deserialize(self.entities.get(entity)?.get(C::ID)?).ok()
    }

    /// Every entity with a readable component `C`, in name order.
    pub fn each<C: Component>(&self) -> impl Iterator<Item = (&str, C)> {
        self.entities.iter().filter_map(|(name, entity)| {
            Some((name.as_str(), C::deserialize(entity.get(C::ID)?).ok()?))
        })
    }

    /// Applies `set` as a whole and returns the change set that undoes it. On any error the
    /// scene is unchanged.
    ///
    /// # Errors
    /// [`REVISION_MISMATCH`], a change that does not fit the scene, or whatever
    /// [`Registry::validate`] reports as an error for the result.
    pub fn apply(
        &mut self,
        registry: &Registry,
        set: &ChangeSet,
    ) -> Result<ChangeSet, Vec<Diagnostic>> {
        if set.base_revision != self.revision {
            let message = format!(
                "the change set is for revision {}, the scene is at {}",
                set.base_revision, self.revision
            );
            return Err(vec![
                Diagnostic::error(REVISION_MISMATCH, message).at("base_revision"),
            ]);
        }
        let mut next = self.clone();
        let mut undo = Vec::new();
        for (i, change) in set.changes.iter().enumerate() {
            let mut inverse = next
                .step(change)
                .map_err(|d| vec![d.at(format!("changes/{i}"))])?;
            // Undo runs backwards: the inverse of the last change comes first.
            inverse.append(&mut undo);
            undo = inverse;
        }
        let found = registry.validate(&next);
        if has_errors(&found) {
            return Err(found);
        }
        next.revision += 1;
        *self = next;
        Ok(ChangeSet {
            base_revision: self.revision,
            changes: undo,
        })
    }

    /// Applies one change and returns the changes that take it back, in order.
    fn step(&mut self, change: &Change) -> Result<Vec<Change>, Diagnostic> {
        let unknown =
            |entity: &str| Diagnostic::error(UNKNOWN_ENTITY, format!("no entity named {entity}"));
        match change {
            Change::EntityCreate { entity } => {
                if self.entities.contains_key(entity) {
                    let message = format!("an entity named {entity} exists");
                    return Err(Diagnostic::error(DUPLICATE_ENTITY, message));
                }
                self.entities.insert(entity.clone(), Entity::new());
                Ok(vec![Change::EntityRemove {
                    entity: entity.clone(),
                }])
            }
            Change::EntityRemove { entity } => {
                let old = self
                    .entities
                    .remove(entity)
                    .ok_or_else(|| unknown(entity))?;
                let mut back = vec![Change::EntityCreate {
                    entity: entity.clone(),
                }];
                back.extend(
                    old.into_iter()
                        .map(|(component, value)| Change::ComponentSet {
                            entity: entity.clone(),
                            component,
                            value,
                        }),
                );
                Ok(back)
            }
            Change::ComponentSet {
                entity,
                component,
                value,
            } => {
                let e = self
                    .entities
                    .get_mut(entity)
                    .ok_or_else(|| unknown(entity))?;
                let (entity, component) = (entity.clone(), component.clone());
                Ok(vec![match e.insert(component.clone(), value.clone()) {
                    Some(value) => Change::ComponentSet {
                        entity,
                        component,
                        value,
                    },
                    None => Change::ComponentRemove { entity, component },
                }])
            }
            Change::ComponentRemove { entity, component } => {
                let e = self
                    .entities
                    .get_mut(entity)
                    .ok_or_else(|| unknown(entity))?;
                let value = e.remove(component).ok_or_else(|| {
                    let message = format!("{entity} has no component {component}");
                    Diagnostic::error(MISSING_COMPONENT, message)
                })?;
                Ok(vec![Change::ComponentSet {
                    entity: entity.clone(),
                    component: component.clone(),
                    value,
                }])
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Serialize, Deserialize, JsonSchema)]
    #[serde(deny_unknown_fields)]
    struct Lamp {
        radius: u32,
    }

    impl Component for Lamp {
        const ID: &'static str = "test.lamp/1";
        fn check(&self, path: &str, out: &mut Vec<Diagnostic>) {
            if self.radius == 0 {
                let code = DiagnosticCode::new("test.lamp.dark");
                out.push(
                    Diagnostic::error(code, "a lamp needs a radius").at(format!("{path}/radius")),
                );
            }
        }
    }

    fn registry() -> Registry {
        let mut r = Registry::default();
        r.register::<Lamp>();
        r
    }

    fn set(entity: &str, value: Value) -> Change {
        Change::ComponentSet {
            entity: entity.into(),
            component: Lamp::ID.into(),
            value,
        }
    }

    fn create(entity: &str) -> Change {
        Change::EntityCreate {
            entity: entity.into(),
        }
    }

    fn codes(found: &[Diagnostic]) -> Vec<(&str, Option<&str>)> {
        found
            .iter()
            .map(|d| (d.code.as_str(), d.path.as_deref()))
            .collect()
    }

    #[test]
    fn a_change_set_applies_as_a_whole_and_its_answer_undoes_it() {
        let registry = registry();
        let mut scene = Scene::default();
        let first = ChangeSet {
            base_revision: 0,
            changes: vec![create("a"), set("a", json!({ "radius": 3 })), create("b")],
        };
        let undo_first = scene.apply(&registry, &first).unwrap();
        assert_eq!(scene.revision, 1);
        assert_eq!(scene.get::<Lamp>("a").unwrap().radius, 3);
        let after_first = scene.clone();

        let second = ChangeSet {
            base_revision: 1,
            changes: vec![
                set("a", json!({ "radius": 9 })),
                Change::EntityRemove { entity: "a".into() },
                create("a"),
                set("b", json!({ "radius": 1 })),
            ],
        };
        let undo_second = scene.apply(&registry, &second).unwrap();
        assert_eq!(scene.revision, 2);
        assert!(scene.get::<Lamp>("a").is_none());
        assert_eq!(
            scene.each::<Lamp>().map(|(n, _)| n).collect::<Vec<_>>(),
            ["b"]
        );

        // Undo is a change set like any other: it moves the revision on and restores the
        // content.
        scene.apply(&registry, &undo_second).unwrap();
        assert_eq!(scene.entities, after_first.entities);
        scene.revision = 1;
        scene.apply(&registry, &undo_first).unwrap();
        assert!(scene.entities.is_empty());
    }

    #[test]
    fn a_failing_change_set_leaves_the_scene_alone_and_says_where() {
        let registry = registry();
        let mut scene = Scene::default();
        let base = ChangeSet {
            base_revision: 0,
            changes: vec![create("a")],
        };
        scene.apply(&registry, &base).unwrap();
        let before = scene.clone();
        let cases: [(Vec<Change>, &str, &str); 6] = [
            (
                vec![create("a")],
                "knorke.scene.duplicate_entity",
                "changes/0",
            ),
            (
                vec![create("b"), set("c", json!({ "radius": 1 }))],
                "knorke.scene.unknown_entity",
                "changes/1",
            ),
            (
                vec![Change::ComponentRemove {
                    entity: "a".into(),
                    component: Lamp::ID.into(),
                }],
                "knorke.scene.missing_component",
                "changes/0",
            ),
            (
                vec![set("a", json!({ "radius": "wide" }))],
                "knorke.scene.invalid_component",
                "entities/a/components/test.lamp/1",
            ),
            (
                vec![set("a", json!({ "radius": 0 }))],
                "test.lamp.dark",
                "entities/a/components/test.lamp/1/radius",
            ),
            (
                vec![Change::ComponentSet {
                    entity: "a".into(),
                    component: "test.horn/1".into(),
                    value: json!({}),
                }],
                "knorke.scene.unknown_component",
                "entities/a/components/test.horn/1",
            ),
        ];
        for (changes, code, path) in cases {
            let set = ChangeSet {
                base_revision: 1,
                changes,
            };
            let found = scene.apply(&registry, &set).unwrap_err();
            assert_eq!(codes(&found), [(code, Some(path))]);
            assert_eq!(scene, before);
        }
        let stale = ChangeSet {
            base_revision: 0,
            changes: vec![],
        };
        let found = scene.apply(&registry, &stale).unwrap_err();
        assert_eq!(
            codes(&found),
            [("knorke.scene.revision_mismatch", Some("base_revision"))]
        );
    }

    #[test]
    fn the_text_round_trips_and_the_type_describes_itself() {
        let registry = registry();
        let mut scene = Scene::default();
        let changes = vec![create("lamp"), set("lamp", json!({ "radius": 4 }))];
        let set = ChangeSet {
            base_revision: 0,
            changes,
        };
        scene.apply(&registry, &set).unwrap();
        let text = scene.to_text();
        assert_eq!(Scene::from_text(&text).unwrap(), scene);
        assert!(text.ends_with("}\n"));
        assert_eq!(
            Scene::from_text("{").unwrap_err().code.as_str(),
            "knorke.scene.malformed"
        );
        let other = Scene {
            schema: "knorke.scene/2".into(),
            ..Scene::default()
        };
        assert_eq!(
            codes(&registry.validate(&other)),
            [("knorke.scene.unknown_schema", Some("schema"))]
        );

        let schemas = registry.schemas();
        let radius = &schemas["test.lamp/1"]["properties"]["radius"];
        assert_eq!(radius["type"], "integer");
        // A change set reads the way the plan writes it.
        let change: Change =
            serde_json::from_value(json!({ "kind": "entity.create", "entity": "x" })).unwrap();
        assert_eq!(change, create("x"));
    }
}
