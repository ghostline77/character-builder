//! Native behaviour options and source-backed NPC presets.
use anyhow::{Context, Result};
use ghostline_red::{
    schema::RedSchema,
    tweakdb::{RecordTypes, TweakDb, TweakId, TweakNames, read_names},
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
    sync::OnceLock,
};

pub fn catalogue() -> Result<&'static Value> {
    static DATA: OnceLock<Result<Value, String>> = OnceLock::new();
    DATA.get_or_init(|| {
        let path = crate::templates::data_path("behaviour.json").map_err(|e| e.to_string())?;
        serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())
    })
    .as_ref()
    .map_err(|e| anyhow::anyhow!(e.clone()))
}

pub fn preset(id: &str) -> Result<Option<&'static Value>> {
    Ok(catalogue()?["presets"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|p| p["id"] == id))
}

/// Rebuilds portable behaviour choices from native character records and names.
pub fn generate(game: &Path, schema: Option<&Path>, out: &Path) -> Result<()> {
    let schema = RedSchema::load(Some(game), schema)?;
    let cache = game.join("r6/cache/tweakdb_ep1.bin");
    let db = TweakDb::from_bytes(&fs::read(&cache)?)?;
    let dictionary = game.join("bin/x64/plugins/cyber_engine_tweaks/tweakdb/tweakdbstr.kark");
    let names = TweakNames::resolve(
        &db,
        read_names(&fs::read(&dictionary).context("native TweakDB name dictionary missing")?)?,
    );
    let types = RecordTypes::new(&schema);
    let fields = [
        "actionMap",
        "archetypeData",
        "baseAttitudeGroup",
        "reactionPreset",
        "rarity",
    ];
    let mut options: BTreeMap<&str, BTreeSet<String>> =
        fields.iter().map(|&key| (key, BTreeSet::new())).collect();
    let mut presets = BTreeMap::new();
    let mut unknown = BTreeMap::<String, usize>::new();
    for (&record, &kind) in &db.records {
        if !types.is_a(kind, "gamedataCharacter_Record") {
            continue;
        }
        let Some(source) = names.records.get(&record) else {
            continue;
        };
        let mut values = BTreeMap::new();
        for field in fields {
            let Some(flat) = db.property(record, field) else {
                continue;
            };
            let value = if flat.native_type == "TweakDBID" {
                let id = flat
                    .value
                    .as_str()
                    .and_then(|s| s.parse::<u64>().ok())
                    .or_else(|| flat.value.as_u64())
                    .unwrap_or(0);
                if id == 0 {
                    continue;
                }
                let Some(value) = names.records.get(&TweakId(id)) else {
                    *unknown.entry(field.into()).or_default() += 1;
                    continue;
                };
                value.clone()
            } else if let Some(value) = flat
                .value
                .as_str()
                .filter(|s| !s.is_empty() && *s != "None")
            {
                value.to_owned()
            } else {
                continue;
            };
            options
                .get_mut(field)
                .expect("indexed field")
                .insert(value.clone());
            values.insert(field.to_owned(), value);
        }
        let Some(archetype) = values.get("archetypeData") else {
            continue;
        };
        if !values.contains_key("actionMap") {
            continue;
        }
        let key = serde_json::to_string(&values)?;
        let label = format!(
            "{} · {} · {}",
            archetype
                .strip_prefix("ArchetypeData.")
                .unwrap_or(archetype),
            values["actionMap"],
            values
                .get("rarity")
                .map(String::as_str)
                .unwrap_or("default")
        );
        presets.entry(key).or_insert_with(||json!({"id":format!("native:{source}"),"label":label,"source_record":source,"values":values}));
    }
    // Include all native action maps and archetypes, even if a character does
    // not currently reference them in the selected cache.
    for (&id, name) in &names.records {
        let Some(&kind) = db.records.get(&id) else {
            continue;
        };
        for (class, field) in [
            ("gamedataAIActionMap_Record", "actionMap"),
            ("gamedataArchetypeData_Record", "archetypeData"),
            ("gamedataReactionPreset_Record", "reactionPreset"),
            ("gamedataNPCRarity_Record", "rarity"),
        ] {
            if types.is_a(kind, class) {
                options
                    .get_mut(field)
                    .expect("indexed field")
                    .insert(name.clone());
            }
        }
    }
    let mut presets: Vec<_> = presets.into_values().collect();
    presets.sort_by_key(|p| p["label"].as_str().unwrap_or("").to_lowercase());
    let data = json!({"format":"ghostline-npc-behaviour","version":1,"sources":{"cache":"tweakdb_ep1.bin","names":"tweakdbstr.kark"},"options":options,"presets":presets,"unresolved_references":unknown});
    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(out, serde_json::to_vec_pretty(&data)?)?;
    println!(
        "{}",
        json!({"presets":data["presets"].as_array().map(Vec::len),"options":data["options"].as_object().map(|o|o.iter().map(|(k,v)|(k,v.as_array().map(Vec::len))).collect::<BTreeMap<_,_>>()),"unresolved_references":data["unresolved_references"],"output":out})
    );
    Ok(())
}
