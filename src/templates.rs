use anyhow::{Context, Result};
use include_dir::{Dir, include_dir};
use serde::Serialize;

#[derive(Clone, Copy, Serialize)]
pub struct Template {
    pub id: &'static str,
    pub label: &'static str,
    pub frame: &'static str,
    pub shape_max: u32,
    pub head_mode: &'static str,
    pub rig: &'static str,
    pub root: &'static str,
}

pub const TEMPLATES: &[Template] = &[
    Template {
        id: "female_average",
        label: "Female · player frame",
        frame: "female",
        shape_max: 22,
        head_mode: "native",
        rig: "base\\characters\\base_entities\\woman_base\\woman_base.rig",
        root: "npv-female",
    },
    Template {
        id: "male_average",
        label: "Male · player frame",
        frame: "male",
        shape_max: 21,
        head_mode: "native",
        rig: "base\\characters\\base_entities\\man_base\\man_base.rig",
        root: "npv-male",
    },
    Template {
        id: "female_average_npc",
        label: "Female · average NPC",
        frame: "female",
        shape_max: 22,
        head_mode: "transfer",
        rig: "base\\characters\\base_entities\\woman_base\\woman_base.rig",
        root: "npc-female_average_npc",
    },
    Template {
        id: "male_average_npc",
        label: "Male · average NPC",
        frame: "male",
        shape_max: 21,
        head_mode: "transfer",
        rig: "base\\characters\\base_entities\\man_base\\man_base.rig",
        root: "npc-male_average_npc",
    },
    Template {
        id: "female_big",
        label: "Female · big NPC",
        frame: "female",
        shape_max: 22,
        head_mode: "transfer",
        rig: "base\\characters\\base_entities\\man_big\\man_big.rig",
        root: "npc-female_big",
    },
    Template {
        id: "male_big",
        label: "Male · big NPC",
        frame: "male",
        shape_max: 21,
        head_mode: "transfer",
        rig: "base\\characters\\base_entities\\man_big\\man_big.rig",
        root: "npc-male_big",
    },
    Template {
        id: "female_fat",
        label: "Female · fat NPC",
        frame: "female",
        shape_max: 22,
        head_mode: "transfer",
        rig: "base\\characters\\base_entities\\woman_base\\woman_base.rig",
        root: "npc-female_fat",
    },
    Template {
        id: "male_fat",
        label: "Male · fat NPC",
        frame: "male",
        shape_max: 21,
        head_mode: "transfer",
        rig: "base\\characters\\base_entities\\man_fat\\man_fat.rig",
        root: "npc-male_fat",
    },
];

pub fn get(id: &str) -> Result<Template> {
    TEMPLATES
        .iter()
        .find(|t| t.id == id)
        .copied()
        .ok_or_else(|| anyhow::anyhow!("unknown character template: {id}"))
}

pub fn shell(template: Template, kind: &str) -> Result<serde_json::Value> {
    let file = format!("{}.{}-shell.json", template.root, kind);
    data(&format!("templates/{file}"))
}

pub fn data(relative: &str) -> Result<serde_json::Value> {
    static DATA: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/data");
    let override_path = std::env::current_exe()?
        .parent()
        .context("executable has no parent directory")?
        .join("data")
        .join(relative);
    if override_path.is_file() {
        return Ok(serde_json::from_slice(&std::fs::read(override_path)?)?);
    }
    let file = DATA
        .get_file(relative)
        .with_context(|| format!("bundled character data missing: {relative}"))?;
    Ok(serde_json::from_slice(file.contents())?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires GHOSTLINE_LEGACY_CHARACTER_TEMPLATES"]
    fn animated_template_bindings_match_the_original_builder() {
        let reference = std::path::PathBuf::from(
            std::env::var_os("GHOSTLINE_LEGACY_CHARACTER_TEMPLATES")
                .expect("original template directory"),
        );
        for template in TEMPLATES {
            let current = shell(*template, "ent").unwrap();
            let original: serde_json::Value = serde_json::from_slice(
                &std::fs::read(reference.join(format!("{}.ent-shell.json", template.root)))
                    .unwrap(),
            )
            .unwrap();
            let chunks = |value: &serde_json::Value| {
                value["Data"]["RootChunk"]["compiledData"]["Data"]["Chunks"]
                    .as_array()
                    .unwrap()
                    .clone()
            };
            let current = chunks(&current);
            for component in chunks(&original)
                .iter()
                .filter(|component| component["$type"] == "entAnimatedComponent")
            {
                let name = &component["name"]["$value"];
                let actual = current
                    .iter()
                    .find(|candidate| candidate["name"]["$value"] == *name)
                    .unwrap_or_else(|| panic!("{} lost animated component {name}", template.id));
                for pointer in [
                    "/rig/DepotPath/$value",
                    "/graph/DepotPath/$value",
                    "/parentTransform/Data/bindName/$value",
                    "/controlBinding/Data/bindName/$value",
                ] {
                    assert_eq!(
                        actual.pointer(pointer),
                        component.pointer(pointer),
                        "{} component {name} {pointer}",
                        template.id
                    );
                }
            }
        }
    }
}
