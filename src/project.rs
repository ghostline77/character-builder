use std::{collections::BTreeMap, fs, io::Write, path::Path};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

use crate::templates;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Project {
    pub format: String,
    pub version: u32,
    pub id: String,
    pub name: String,
    pub revision: u64,
    pub template: String,
    pub namespace: String,
    pub catalogue: Option<CataloguePin>,
    pub appearances: Vec<Appearance>,
    pub active_appearance: String,
    pub identity: Identity,
    #[serde(default)]
    pub loadout: Vec<String>,
    #[serde(default)]
    pub ai: AiSettings,
    /// Opt in to generated garment deformation and body-coverage masks.
    #[serde(default)]
    pub refit_clothing: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CataloguePin {
    pub format: String,
    pub version: u32,
    pub sources: BTreeMap<String, String>,
    /// Identity of the source/control/selection contract, independent of rendered media.
    pub identity: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Appearance {
    pub id: String,
    pub name: String,
    pub choices: BTreeMap<String, String>,
    pub equipment: BTreeMap<String, String>,
    pub shapes: BTreeMap<String, u32>,
    /// Exact native mesh variants retained independently of inventory item variants.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub mesh_overrides: BTreeMap<String, MeshOverride>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub native_parts: Vec<NativePart>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_head: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skin_tone: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_outfit: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MeshOverride {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub appearance: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chunk_mask: Option<String>,
}

/// A native mesh with an optional animated rig, for assets outside inventory records.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NativePart {
    pub id: String,
    pub mesh: String,
    pub appearance: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rig: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shadow: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub visual_tags: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    pub record: String,
    /// Published localization key used for the character's in-game name.
    #[serde(default)]
    pub display_name: String,
    pub voice_tag: String,
    pub affiliation: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct AiSettings {
    pub profile: String,
    pub action_map: String,
    pub archetype_data: String,
    pub attitude_group: String,
    pub reaction_preset: String,
    pub rarity: String,
    pub power_level: Option<u32>,
    pub health_multiplier: Option<f32>,
}

impl Default for AiSettings {
    fn default() -> Self {
        Self {
            profile: "quest".into(),
            action_map: String::new(),
            archetype_data: String::new(),
            attitude_group: String::new(),
            reaction_preset: String::new(),
            rarity: String::new(),
            power_level: None,
            health_multiplier: None,
        }
    }
}

impl Project {
    pub fn new(id: &str, template: &str) -> Result<Self> {
        safe_id(id)?;
        templates::get(template)?;
        Ok(Self {
            format: "ghostline-character".into(),
            version: 1,
            id: id.into(),
            name: id.replace('_', " "),
            revision: 0,
            template: template.into(),
            namespace: format!("mod\\characters\\{id}"),
            catalogue: None,
            loadout: Vec::new(),
            ai: AiSettings::default(),
            refit_clothing: false,
            appearances: vec![Appearance {
                id: "default".into(),
                name: "Default".into(),
                choices: BTreeMap::new(),
                equipment: BTreeMap::new(),
                shapes: BTreeMap::new(),
                mesh_overrides: BTreeMap::new(),
                native_parts: Vec::new(),
                native_head: None,
                skin_tone: None,
                native_outfit: None,
            }],
            active_appearance: "default".into(),
            identity: Identity {
                record: format!("Character.{id}"),
                display_name: String::new(),
                voice_tag: String::new(),
                affiliation: String::new(),
            },
        })
    }

    pub fn check(&self) -> Result<()> {
        ensure!(
            self.format == "ghostline-character" && self.version == 1,
            "unsupported character format/version"
        );
        safe_id(&self.id)?;
        ensure!(
            ["", "quest", "combat", "cyberpsycho_netrunner"].contains(&self.ai.profile.as_str())
                || (self.ai.profile.starts_with("native:")
                    && crate::ai::preset(&self.ai.profile)?.is_some()),
            "unknown NPC behavior profile"
        );
        if let Some(health) = self.ai.health_multiplier {
            ensure!(
                health.is_finite() && health > 0.,
                "health multiplier must be positive and finite"
            );
        }
        ensure!(
            self.ai
                .power_level
                .is_none_or(|level| level > 0 && level <= 100),
            "power level must be 1..100"
        );
        let template = templates::get(&self.template)?;
        ensure!(!self.name.trim().is_empty(), "character name is empty");
        ensure!(
            self.identity.record.starts_with("Character.")
                && self
                    .identity
                    .record
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.')),
            "character record must be a Character identifier"
        );
        ensure!(
            self.namespace.starts_with("mod\\")
                && self.namespace.split('\\').all(|part| !part.is_empty()
                    && part != "."
                    && part != ".."
                    && part
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')),
            "namespace must be a mod depot path"
        );
        ensure!(
            !self.appearances.is_empty(),
            "character needs an appearance"
        );
        let mut ids = std::collections::BTreeSet::new();
        for appearance in &self.appearances {
            let mut parts = std::collections::BTreeSet::new();
            for part in &appearance.native_parts {
                safe_id(&part.id)?;
                ensure!(parts.insert(&part.id), "duplicate native part identity");
                ensure!(
                    !part.appearance.trim().is_empty(),
                    "native part needs a named mesh appearance"
                );
                ensure!(
                    part.rig.is_some() == part.graph.is_some(),
                    "native animation requires both rig and graph"
                );
                for (path, extension) in std::iter::once((&part.mesh, ".mesh"))
                    .chain(part.rig.iter().map(|p| (p, ".rig")))
                    .chain(part.graph.iter().map(|p| (p, ".animgraph")))
                    .chain(part.shadow.iter().map(|p| (p, ".mesh")))
                {
                    ensure!(
                        (path.starts_with("base\\") || path.starts_with("ep1\\"))
                            && path.ends_with(extension)
                            && path.split('\\').all(|p| !p.is_empty()
                                && p != ".."
                                && p != "."
                                && !p.contains(['/', ':'])),
                        "invalid native part resource {path}"
                    );
                }
            }
            for (mesh, value) in &appearance.mesh_overrides {
                ensure!(
                    mesh.parse::<u64>().is_ok_and(|value| value > 0)
                        || ((mesh.starts_with("base\\") || mesh.starts_with("ep1\\"))
                            && mesh.ends_with(".mesh")
                            && mesh.split('\\').all(|part| !part.is_empty()
                                && part != "."
                                && part != ".."
                                && !part.contains(['/', ':']))),
                    "mesh override must identify a native game mesh"
                );
                ensure!(
                    value.resource.is_some()
                        || value.appearance.is_some()
                        || value.chunk_mask.is_some(),
                    "empty mesh override for {mesh}"
                );
                if let Some(path) = &value.resource {
                    ensure!(
                        (path.starts_with("base\\") || path.starts_with("ep1\\"))
                            && path.ends_with(".mesh")
                            && path.split('\\').all(|p| !p.is_empty()
                                && p != ".."
                                && p != "."
                                && !p.contains(['/', ':'])),
                        "invalid replacement mesh {path}"
                    );
                }
                ensure!(
                    value
                        .appearance
                        .as_ref()
                        .is_none_or(|name| !name.trim().is_empty()),
                    "empty mesh appearance for {mesh}"
                );
                ensure!(
                    value
                        .chunk_mask
                        .as_ref()
                        .is_none_or(|mask| mask.parse::<u64>().is_ok()),
                    "mesh chunk mask must be uint64"
                );
            }
            if let Some(tone) = &appearance.skin_tone {
                ensure!(
                    [
                        "01_ca_pale",
                        "02_ca_limestone",
                        "03_ca_senna",
                        "04_ca_almond",
                        "05_bl_espresso",
                        "06_bl_dark"
                    ]
                    .contains(&tone.as_str()),
                    "unknown native skin tone: {tone}"
                );
            }
            safe_id(&appearance.id)?;
            ensure!(ids.insert(&appearance.id), "duplicate appearance identity");
            for (name, value) in &appearance.shapes {
                ensure!(
                    ["eyes", "nose", "mouth", "jaw", "ears"].contains(&name.as_str()),
                    "unknown face region: {name}"
                );
                ensure!(
                    (1..=template.shape_max).contains(value),
                    "{name} shape must be 1..{}",
                    template.shape_max
                );
            }
        }
        ensure!(
            ids.contains(&self.active_appearance),
            "active appearance is absent"
        );
        Ok(())
    }

    pub fn appearance(&self) -> &Appearance {
        self.appearances
            .iter()
            .find(|a| a.id == self.active_appearance)
            .expect("validated active appearance")
    }
}

pub fn safe_id(id: &str) -> Result<()> {
    ensure!(
        !id.is_empty()
            && id.len() <= 80
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'),
        "identity must contain 1..80 letters, numbers, underscores or hyphens"
    );
    Ok(())
}

pub fn read(path: &Path) -> Result<Project> {
    let project: Project = serde_json::from_slice(
        &fs::read(path).with_context(|| format!("read {}", path.display()))?,
    )?;
    project.check()?;
    Ok(project)
}

pub fn write(path: &Path, project: &Project) -> Result<()> {
    project.check()?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(&mut file, project)?;
    file.write_all(b"\n")?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|error| error.error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refitting_is_opt_in_and_the_choice_survives_saving() {
        let project = Project::new("refit_toggle", "female_average").unwrap();
        assert!(!project.refit_clothing);
        let mut saved = serde_json::to_value(&project).unwrap();
        saved.as_object_mut().unwrap().remove("refit_clothing");
        let loaded: Project = serde_json::from_value(saved.clone()).unwrap();
        assert!(!loaded.refit_clothing);
        saved["refit_clothing"] = serde_json::json!(true);
        let enabled: Project = serde_json::from_value(saved).unwrap();
        assert!(enabled.refit_clothing);
        assert_eq!(
            serde_json::from_str::<Project>(&serde_json::to_string(&enabled).unwrap()).unwrap(),
            enabled
        );
    }

    #[test]
    fn saved_projects_preserve_appearances_and_exact_native_choices() {
        let directory = tempfile::tempdir().unwrap();
        for template in templates::TEMPLATES {
            let mut project = Project::new("round_trip", template.id).unwrap();
            project.appearances[0].choices.insert(
                "native/control/7".into(),
                "native/control/7/definitions/22".into(),
            );
            let mut alternate = project.appearances[0].clone();
            alternate.id = "night".into();
            alternate.shapes.insert("eyes".into(), template.shape_max);
            project.appearances.push(alternate);
            project.active_appearance = "night".into();
            project.appearances[1].skin_tone = Some("05_bl_espresso".into());
            project.loadout.push("item:weapon".into());
            project.ai.profile = "cyberpsycho_netrunner".into();
            project.ai.health_multiplier = Some(3.0);
            project.ai.power_level = Some(60);
            let path = directory.path().join(format!("{}.json", template.id));
            write(&path, &project).unwrap();
            assert_eq!(read(&path).unwrap(), project);
        }
    }

    #[test]
    fn invalid_projects_do_not_escape_their_namespace_or_hide_appearance_conflicts() {
        assert!(Project::new("../outside", "female_average").is_err());
        let mut project = Project::new("valid", "female_average").unwrap();
        project.namespace = "mod\\..\\outside".into();
        assert!(project.check().is_err());
        project.namespace = "mod\\characters\\valid".into();
        project.appearances.push(project.appearances[0].clone());
        assert!(project.check().is_err());
    }
}
