use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Component, Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::{
    project::{CataloguePin, Project},
    templates,
};

pub struct Catalogue {
    pub root: PathBuf,
    pub pin: CataloguePin,
    pub selections: BTreeMap<String, Value>,
    pub controls: BTreeMap<String, Value>,
    pub choices: BTreeMap<String, Value>,
    pub entries: Vec<Value>,
    weapon_selections: BTreeMap<String, Vec<String>>,
    search_text: Vec<String>,
}

#[derive(Serialize)]
pub struct Validation {
    pub ok: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Serialize)]
pub struct LoadedProject {
    pub project: Project,
    pub adjustments: Vec<String>,
    pub validation: Validation,
}

pub fn relative(root: &Path, name: &str) -> Result<PathBuf> {
    let name = Path::new(name);
    ensure!(
        !name.as_os_str().is_empty()
            && name.components().all(|p| matches!(p, Component::Normal(_))),
        "unsafe package path"
    );
    let candidate = root.join(name).canonicalize()?;
    ensure!(
        candidate.starts_with(root.canonicalize()?),
        "package path escapes its root"
    );
    Ok(candidate)
}

fn collection(root: &Path, descriptor: &Value, name: &str) -> Result<Vec<Value>> {
    let mut rows = Vec::new();
    for file in descriptor["collections"][name]
        .as_array()
        .context("missing catalogue collection")?
    {
        let path = relative(root, file.as_str().context("invalid shard path")?)?;
        let shard: Vec<Value> = serde_json::from_slice(
            &fs::read(&path).with_context(|| format!("read {}", path.display()))?,
        )?;
        rows.extend(shard);
    }
    Ok(rows)
}

fn by_id(rows: Vec<Value>) -> Result<BTreeMap<String, Value>> {
    let mut entries = BTreeMap::new();
    for row in rows {
        let id = row["id"]
            .as_str()
            .context("catalogue entry has no identity")?
            .to_owned();
        ensure!(
            entries.insert(id, row).is_none(),
            "duplicate catalogue identity"
        );
    }
    Ok(entries)
}

pub fn resource_path(resource: &Value) -> Option<&str> {
    resource["Path"].as_str()
}

/// Reads an explicit native appearance condition, never a mesh filename guess.
fn selection_frame(selection: &Value) -> Option<&'static str> {
    selection["appearance"]
        .as_str()?
        .split('&')
        .skip(1)
        .find_map(|condition| match condition {
            "Female" | "gender=f" => Some("female"),
            "Male" | "gender=m" => Some("male"),
            _ => None,
        })
}

/// Individual native garments are catalogued independently of inventory records.
/// The filename's body-family token is an authored fit target, not a gender guess.
fn garment_entry(selection: &Value) -> Option<Value> {
    if selection["context"]["clothing"] != true {
        return None;
    }
    let path = selection["resource"]["Path"].as_str()?;
    if path
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|p| p.eq_ignore_ascii_case("fpp"))
    {
        return None;
    }
    let filename = path.rsplit('\\').next()?;
    let stem = filename.rsplit_once('.')?.0;
    let mut tokens = stem.split('_');
    let slot = tokens.next()?;
    let overlay = stem
        .split('_')
        .any(|token| matches!(token, "dec" | "decal" | "decals"));
    let category = match slot {
        _ if overlay => "Clo_Decals",
        "t1" => "Clo_InnerChest",
        "t2" => "Clo_OuterChest",
        "l1" => "Clo_Legs",
        "s1" => "Clo_Feet",
        "h1" | "h2" => "Clo_Head",
        "f1" => "Clo_Face",
        "g1" => "Clo_Hands",
        "i1" => "Clo_Accessories",
        _ => return None,
    };
    tokens.next()?;
    let family = tokens.next()?;
    let (frame, templates): (&str, &[&str]) = match family {
        "wa" | "pwa" => ("female", &["female_average", "female_average_npc"]),
        "ma" | "pma" => ("male", &["male_average", "male_average_npc"]),
        "mb" | "mba" => ("male", &["male_big"]),
        "wba" => ("female", &["female_big"]),
        "mf" => ("male", &["male_fat"]),
        "waf" => ("female", &["female_fat"]),
        // Child and special-purpose body families have no builder template.
        _ => return None,
    };
    let label = tokens
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    Some(
        json!({"id":selection["id"],"label":label,"category":category,
        "kind":"equipment","frame":frame,"body_family":family,"templates":templates,
        "garment_style":path,"variant_label":selection["appearance"],
        "resource":selection["resource"],"appearance":selection["appearance"],
        "resolution":selection["resolution"],"preview_state":selection["preview_state"],"media":selection["media"]}),
    )
}

/// Exact native CC root identities determine the creator frame, not mesh filename guesses.
pub(crate) fn creator_frame(source: &Value) -> Option<&'static str> {
    match resource_path(source)? {
        "base\\gameplay\\gui\\fullscreen\\main_menu\\female_cco.inkcharcustomization"
        | "ep1\\gameplay\\gui\\fullscreen\\main_menu\\female_cco_ep1.inkcharcustomization" => {
            Some("female")
        }
        "base\\gameplay\\gui\\fullscreen\\main_menu\\male_cco.inkcharcustomization"
        | "ep1\\gameplay\\gui\\fullscreen\\main_menu\\male_cco_ep1.inkcharcustomization" => {
            Some("male")
        }
        _ => None,
    }
}

impl Catalogue {
    pub fn select_choice(&self, project: &mut Project, id: &str) -> Result<()> {
        project.check()?;
        let choice = self.choices.get(id).context("native choice is absent")?;
        let control_id = choice["control"]
            .as_str()
            .context("choice control is absent")?;
        let control = self
            .controls
            .get(control_id)
            .context("native control is absent")?;
        let slot = control["native"]["ui_slot"].as_str();
        let native_npc = templates::get(&project.template)?.head_mode == "transfer";
        let appearance = project
            .appearances
            .iter_mut()
            .find(|a| a.id == project.active_appearance)
            .context("active appearance is absent")?;
        if let Some(slot) = slot {
            appearance.choices.retain(|key, _| {
                self.controls.get(key).is_none_or(|c| {
                    let other = c["native"]["ui_slot"].as_str();
                    other != Some(slot)
                        && !(matches!(slot, "flat_feet" | "lifted_feet")
                            && matches!(other, Some("flat_feet" | "lifted_feet")))
                })
            });
        }
        appearance.choices.insert(control_id.into(), id.into());
        if native_npc && slot == Some("skin_type") {
            let selection = choice["selection"]
                .as_str()
                .and_then(|id| self.selections.get(id))
                .context("skin selection absent")?;
            let name = selection["appearance"]
                .as_str()
                .context("skin appearance absent")?;
            let tone=["01_ca_pale","02_ca_limestone","03_ca_senna","04_ca_almond","05_bl_espresso","06_bl_dark"].into_iter().find(|tone|name.ends_with(&format!("__{tone}"))).context("this creator tint is not a native NPC palette; use Skin colour to select a shared head/body tone")?;
            appearance.skin_tone = Some(tone.into());
        }
        // Native linked controls share the indexed definition (for example skin
        // tone across the head, torso, arms and the active foot shape).
        if let Some(link) = control["native"]["data"]["link"]["$value"]
            .as_str()
            .filter(|s| !s.is_empty())
        {
            let index = choice["data"]["index"].as_u64().unwrap_or(0);
            let linked: Vec<_> = appearance
                .choices
                .keys()
                .filter_map(|key| {
                    let other = self.controls.get(key)?;
                    (creator_frame(&other["source"]) == creator_frame(&control["source"])
                        && other["native"]["data"]["link"]["$value"] == link)
                        .then_some(key.clone())
                })
                .collect();
            for key in linked {
                let matching = self
                    .choices
                    .values()
                    .find(|candidate| {
                        candidate["control"] == key
                            && candidate["kind"] == choice["kind"]
                            && candidate["data"]["index"].as_u64().unwrap_or(0) == index
                    })
                    .with_context(|| {
                        format!("linked native control {key} has no definition {index}")
                    })?;
                appearance.choices.insert(
                    key,
                    matching["id"]
                        .as_str()
                        .context("linked choice identity is absent")?
                        .into(),
                );
            }
        }
        // Preserve the native switcher choice that activates this appearance control.
        for parent in self.choices.values().filter(|c| c["kind"] == "options") {
            let activates = parent["data"]["names"].as_array().is_some_and(|names| {
                names
                    .iter()
                    .any(|name| name["$value"] == control["native"]["name"])
            });
            if activates
                && let Some(parent_id) = parent["control"].as_str()
                && let Some(parent_control) = self.controls.get(parent_id)
                && parent_control["source"] == control["source"]
            {
                appearance.choices.insert(
                    parent_id.into(),
                    parent["id"]
                        .as_str()
                        .context("switcher choice has no identity")?
                        .into(),
                );
            }
        }
        let validation = self.validate(project);
        ensure!(validation.ok, "{}", validation.errors.join("; "));
        Ok(())
    }

    pub fn load(root: &Path) -> Result<Self> {
        let root = root.canonicalize()?;
        let descriptor: Value =
            serde_json::from_slice(&fs::read(root.join("data/catalogue.json"))?)?;
        ensure!(
            descriptor["format"] == "ghostline-items" && descriptor["version"] == 1,
            "unsupported items catalogue"
        );
        let selections = by_id(collection(&root, &descriptor, "selections")?)?;
        let controls = by_id(collection(&root, &descriptor, "controls")?)?;
        let choices = by_id(collection(&root, &descriptor, "choices")?)?;
        let sources: BTreeMap<String, String> =
            serde_json::from_value(descriptor["sources"].clone())?;
        let mut hash = Sha256::new();
        hash.update(serde_json::to_vec(&sources)?);
        hash.update(serde_json::to_vec(&controls)?);
        hash.update(serde_json::to_vec(&choices)?);
        for (id, selection) in &selections {
            hash.update(serde_json::to_vec(&(
                id,
                &selection["resource"],
                &selection["appearance"],
                &selection["components"],
            ))?);
        }
        let pin = CataloguePin {
            format: "ghostline-items".into(),
            version: 1,
            sources,
            identity: format!("{:x}", hash.finalize()),
        };
        let mut entries = Vec::new();
        let mut weapon_selections: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut switchers = BTreeMap::new();
        for choice in choices.values().filter(|c| c["kind"] == "options") {
            if let Some(control) = choice["control"].as_str().and_then(|id| controls.get(id)) {
                for name in choice["data"]["names"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|n| n["$value"].as_str())
                {
                    switchers.insert((control["source"].to_string(), name.to_owned()), choice);
                }
            }
        }
        let mut seen = BTreeSet::new();
        for choice in choices.values() {
            let Some(selection_id) = choice["selection"].as_str() else {
                continue;
            };
            let Some(selection) = selections.get(selection_id) else {
                continue;
            };
            let Some(control) = choice["control"].as_str().and_then(|id| controls.get(id)) else {
                continue;
            };
            let native = &control["native"];
            let Some(slot) = native["ui_slot"].as_str() else {
                continue;
            };
            // Perspective companions remain in the source graph, not duplicate picker cards.
            if slot.starts_with("fpp_")
                || slot.ends_with("_fpp")
                || slot.is_empty()
                || matches!(slot, "holstered_data" | "finalSceneBruises")
                || !seen.insert((slot.to_owned(), selection_id.to_owned()))
            {
                continue;
            }
            let switcher = native["name"]
                .as_str()
                .and_then(|name| switchers.get(&(control["source"].to_string(), name.to_owned())));
            let style = switcher.map(|choice| {
                choice["data"]["localizedName"]
                    .as_str()
                    .filter(|label| !label.is_empty() && label.bytes().all(|b| b.is_ascii_digit()))
                    .map(str::to_owned)
                    .unwrap_or_else(|| choice["label"].as_str().unwrap_or("").to_owned())
            });
            entries.push(json!({"id":selection_id,"choice":choice["id"],"control":control["id"],
                "style":style,"switcher":switcher.map(|choice|&choice["id"]),
                "label":choice["label"],"variant_index":choice["data"]["index"],"category":slot,"kind":"customization","frame":creator_frame(&control["source"]),
                "control_name":native["name"],"resource":selection["resource"],"appearance":selection["appearance"],
                "resolution":selection["resolution"],"preview_state":selection["preview_state"],"media":selection["media"]}));
        }
        for item in collection(&root, &descriptor, "items")? {
            let category = item["item_type"].as_str().unwrap_or("");
            if category.starts_with("Wea_Vehicle") {
                continue;
            }
            if category.starts_with("Wea_") {
                if let Some(id) = item["id"].as_str() {
                    for selection in item["selections"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                    {
                        weapon_selections
                            .entry(selection.into())
                            .or_default()
                            .push(id.into());
                    }
                }
                let role = match category {
                    "Wea_SniperRifle" => "sniper",
                    "Wea_Axe" | "Wea_Chainsword" | "Wea_Fists" | "Wea_Hammer" | "Wea_Katana"
                    | "Wea_Knife" | "Wea_LongBlade" | "Wea_Machete" | "Wea_OneHandedClub"
                    | "Wea_TwoHandedClub" | "Wea_Melee" | "Wea_Sword" => "melee",
                    _ => "ranged",
                };
                let media = item["selections"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .filter_map(|id| selections.get(id))
                    .find_map(|selection| {
                        selection["media"]
                            .as_array()
                            .filter(|media| !media.is_empty())
                    })
                    .cloned()
                    .unwrap_or_default();
                entries.push(json!({"id":item["id"],"record":item["native_name"],"label":item["label"],"category":category,"kind":"loadout","role":role,"resolution":"resolved","media":media,"frame":null,"visual_tags":item["properties"]["visualTags"]["value"]}));
                continue;
            }
            if !category.starts_with("Clo_") {
                continue;
            }
            for id in item["selections"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                let Some(selection) = selections.get(id) else {
                    continue;
                };
                // NPCs wear the third-person garment; first-person companions
                // remain available in the source graph, not as picker choices.
                if selection["appearance"].as_str().is_some_and(|appearance| {
                    appearance
                        .split(|ch: char| !ch.is_ascii_alphanumeric())
                        .any(|part| part.eq_ignore_ascii_case("fpp"))
                }) {
                    continue;
                }
                if !seen.insert(("equipment".into(), id.into())) {
                    continue;
                }
                entries.push(json!({"id":id,"record":item["id"],"label":item["label"],"category":item["item_type"],
                    "kind":"equipment","frame":selection_frame(selection),"resource":selection["resource"],"appearance":selection["appearance"],
                    "resolution":selection["resolution"],"preview_state":selection["preview_state"],"media":selection["media"]}));
            }
        }
        for selection in selections.values() {
            if let Some(entry) = garment_entry(selection)
                && seen.insert((
                    "equipment".into(),
                    selection["id"].as_str().unwrap_or_default().into(),
                ))
            {
                entries.push(entry);
            }
        }
        // Alternate scalp meshes activated by the same native style switcher
        // belong to one style. Prefer its ordinary control for each colour.
        entries.sort_by_key(|entry| {
            entry["control_name"]
                .as_str()
                .is_some_and(|name| name.contains("_cyberware"))
        });
        let search_text = entries
            .iter()
            .map(|entry| entry.to_string().to_lowercase())
            .collect();
        Ok(Self {
            root,
            pin,
            selections,
            controls,
            choices,
            entries,
            weapon_selections,
            search_text,
        })
    }

    pub fn search(
        &self,
        query: &str,
        category: &str,
        frame: &str,
        template: &str,
        offset: usize,
        limit: usize,
    ) -> Value {
        let words: Vec<_> = query
            .to_lowercase()
            .split_whitespace()
            .map(str::to_owned)
            .collect();
        let matches: Vec<_> = self
            .entries
            .iter()
            .zip(&self.search_text)
            .filter(|(entry, text)| {
                (category.is_empty()
                    || entry["category"] == category
                    || entry["kind"] == category
                    || (category == "clothing"
                        && (entry["kind"] == "equipment" || entry["kind"] == "npc_clothing")))
                    && (frame.is_empty() || entry["frame"].is_null() || entry["frame"] == frame)
                    && (template.is_empty()
                        || entry["template"].is_null()
                        || entry["template"] == template)
                    && (template.is_empty()
                        || entry["templates"]
                            .as_array()
                            .is_none_or(|allowed| allowed.iter().any(|t| t == template)))
                    && !(entry["kind"] == "equipment"
                        && entry["templates"].is_null()
                        && matches!(
                            template,
                            "female_big" | "male_big" | "female_fat" | "male_fat"
                        ))
                    && words.iter().all(|word| text.contains(word))
            })
            .map(|(entry, _)| entry)
            .collect();
        let mut groups: Vec<Value> = Vec::new();
        let mut indices = BTreeMap::new();
        for entry in matches {
            if entry["kind"] == "customization"
                || entry["kind"] == "loadout"
                || !entry["garment_style"].is_null()
            {
                let key = (
                    format!("{}/{}", entry["kind"], entry["category"]),
                    if !entry["garment_style"].is_null() {
                        entry["garment_style"].to_string()
                    } else if entry["kind"] == "loadout" {
                        entry["label"].to_string()
                    } else {
                        entry
                            .get("switcher")
                            .filter(|value| !value.is_null())
                            .unwrap_or(&entry["control"])
                            .to_string()
                    },
                );
                if let Some(&index) = indices.get(&key) {
                    let group: &mut Value = &mut groups[index];
                    let variants = group["variants"].as_array_mut().expect("variant list");
                    if entry["kind"] == "loadout"
                        || !variants.iter().any(|variant| {
                            !entry["variant_index"].is_null()
                                && variant["variant_index"] == entry["variant_index"]
                        })
                    {
                        variants.push(entry.clone());
                    }
                } else {
                    indices.insert(key, groups.len());
                    let mut group = entry.clone();
                    group["variants"] = json!([entry]);
                    groups.push(group);
                }
            } else {
                groups.push(entry.clone());
            }
        }
        for group in &mut groups {
            if let Some(variants) = group["variants"].as_array_mut() {
                variants.sort_by_cached_key(|variant| {
                    (
                        variant["variant_index"].as_u64().unwrap_or(u64::MAX),
                        variant["record"].as_str().unwrap_or("").to_owned(),
                    )
                });
            }
        }
        groups.sort_by_cached_key(|entry| {
            (
                entry["category"].as_str().unwrap_or("").to_owned(),
                entry["style"]
                    .as_str()
                    .and_then(|style| style.parse::<u32>().ok())
                    .unwrap_or(u32::MAX),
                entry["label"].as_str().unwrap_or("").to_lowercase(),
            )
        });
        json!({"total":groups.len(),"offset":offset,"entries":groups.into_iter().skip(offset).take(limit.min(120)).collect::<Vec<_>>()})
    }

    pub fn categories(&self) -> Vec<String> {
        self.entries
            .iter()
            .filter_map(|e| e["category"].as_str().map(str::to_owned))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    pub fn defaults(&self, project: &mut Project) -> Result<()> {
        let frame = templates::get(&project.template)?.frame;
        let names = [
            "skin_type_01",
            "body_color",
            "eyes_color",
            if frame == "female" {
                "h_default_arms_colors_tpp"
            } else {
                "h_default_arms_colors"
            },
        ];
        for name in names {
            if let Some(control) = self
                .controls
                .values()
                .find(|c| creator_frame(&c["source"]) == Some(frame) && c["native"]["name"] == name)
                && let Some(choice) = self.choices.values().find(|c| {
                    c["control"] == control["id"]
                        && c["selection"].as_str().is_some_and(|id| {
                            self.selections
                                .get(id)
                                .is_some_and(|s| s["resolution"] == "resolved")
                        })
                })
            {
                project.appearances[0].choices.insert(
                    control["id"]
                        .as_str()
                        .context("control identity missing")?
                        .into(),
                    choice["id"]
                        .as_str()
                        .context("choice identity missing")?
                        .into(),
                );
            }
        }
        for slot in ["flat_feet", "eyebrows_color", "eyelash_color", "teeth"] {
            if let Some(control) = self.controls.values().find(|c| {
                creator_frame(&c["source"]) == Some(frame) && c["native"]["ui_slot"] == slot
            }) && let Some(choice) = self.choices.values().find(|c| {
                c["control"] == control["id"]
                    && c["selection"].as_str().is_some_and(|id| {
                        self.selections
                            .get(id)
                            .is_some_and(|s| s["resolution"] == "resolved")
                    })
            }) {
                project.appearances[0].choices.insert(
                    control["id"]
                        .as_str()
                        .context("baseline control identity is absent")?
                        .into(),
                    choice["id"]
                        .as_str()
                        .context("baseline choice identity is absent")?
                        .into(),
                );
            }
        }
        project.catalogue = Some(self.pin.clone());
        Ok(())
    }

    pub fn selected<'a>(&'a self, project: &Project) -> Result<Vec<&'a Value>> {
        project.check()?;
        let mut selections = BTreeMap::new();
        for (control, id) in &project.appearance().choices {
            let choice = self
                .choices
                .get(id)
                .with_context(|| format!("missing choice: {id}"))?;
            ensure!(
                choice["control"] == *control,
                "choice belongs to another control"
            );
            if let Some(id) = choice["selection"].as_str() {
                selections.insert(
                    id,
                    self.selections
                        .get(id)
                        .context("choice selection is absent")?,
                );
            }
        }
        for (category, id) in &project.appearance().equipment {
            ensure!(
                category.starts_with("Clo_"),
                "{category} is not wearable clothing; choose weapons in Weapons / loadout"
            );
            selections.insert(
                id.as_str(),
                self.selections
                    .get(id)
                    .with_context(|| format!("missing equipment selection: {id}"))?,
            );
        }
        Ok(selections.into_values().collect())
    }

    /// Opens an authored project against current catalogue references, repairing
    /// known incomplete baselines and the former visual-weapon representation.
    pub fn prepare_project(&self, mut project: Project) -> Result<LoadedProject> {
        project.check()?;
        let mut adjustments = Vec::new();
        if project.catalogue.as_ref() != Some(&self.pin) {
            adjustments.push("Updated catalogue reference; existing selections were checked against the current catalogue.".into());
            project.catalogue = Some(self.pin.clone());
        }
        let mut baseline = Project::new(&project.id, &project.template)?;
        self.defaults(&mut baseline)?;
        for appearance in &mut project.appearances {
            let has_feet = appearance
                .choices
                .keys()
                .filter_map(|id| self.controls.get(id))
                .any(|control| {
                    matches!(
                        control["native"]["ui_slot"].as_str(),
                        Some("flat_feet" | "lifted_feet")
                    )
                });
            if templates::get(&project.template)?.frame == "female"
                && !has_feet
                && let Some((control, choice)) =
                    baseline.appearance().choices.iter().find(|(id, _)| {
                        self.controls
                            .get(*id)
                            .is_some_and(|c| c["native"]["ui_slot"] == "flat_feet")
                    })
            {
                // Preserve the selected linked body colour when supplying
                // the newly required foot geometry.
                let native_control = &self.controls[control];
                let link = &native_control["native"]["data"]["link"]["$value"];
                let index = appearance.choices.iter().find_map(|(id, choice)| {
                    let control = self.controls.get(id)?;
                    (control["native"]["ui_slot"] == "body_color"
                        && !link.is_null()
                        && control["native"]["data"]["link"]["$value"] == *link)
                        .then(|| {
                            self.choices
                                .get(choice)
                                .and_then(|c| c["data"]["index"].as_u64())
                                .unwrap_or(0)
                        })
                });
                let choice = index
                    .and_then(|index| {
                        self.choices
                            .values()
                            .find(|c| {
                                c["control"] == *control
                                    && c["data"]["index"].as_u64().unwrap_or(0) == index
                                    && c["selection"].as_str().is_some_and(|id| {
                                        self.selections
                                            .get(id)
                                            .is_some_and(|s| s["resolution"] == "resolved")
                                    })
                            })
                            .and_then(|c| c["id"].as_str())
                            .map(str::to_owned)
                    })
                    .unwrap_or_else(|| choice.clone());
                appearance.choices.insert(control.clone(), choice);
                adjustments.push(format!("{}: added the missing lower legs and feet baseline, preserving the linked body colour.",appearance.name));
            }
            let weapons: Vec<_> = appearance
                .equipment
                .iter()
                .filter(|(slot, _)| slot.starts_with("Wea_"))
                .map(|(slot, id)| (slot.clone(), id.clone()))
                .collect();
            for (slot, selection) in weapons {
                let mut candidates: Vec<_> = self
                    .entries
                    .iter()
                    .filter(|entry| {
                        entry["kind"] == "loadout"
                            && entry["category"] == slot
                            && (entry["id"] == selection
                                || self
                                    .weapon_selections
                                    .get(&selection)
                                    .is_some_and(|ids| ids.iter().any(|id| entry["id"] == *id)))
                    })
                    .filter(|entry| {
                        entry["record"]
                            .as_str()
                            .is_some_and(|r| r.starts_with("Items."))
                    })
                    .collect();
                candidates.sort_by_key(|entry| {
                    let name = entry["record"].as_str().unwrap_or("");
                    let appearance = selection.rsplit_once('#').map(|(_, name)| name);
                    let matching_tags = entry["visual_tags"].as_array().is_some_and(|tags| {
                        tags.iter().filter_map(Value::as_str).any(|tag| {
                            appearance.is_some_and(|name| tag.eq_ignore_ascii_case(name))
                        })
                    });
                    (
                        !matching_tags,
                        !name.starts_with("Items.Preset_"),
                        !name.ends_with("_Default"),
                        name.contains("Left_Hand"),
                        name.len(),
                        name.to_owned(),
                    )
                });
                if let Some(entry) = candidates.first() {
                    let id = entry["id"]
                        .as_str()
                        .context("loadout identity missing")?
                        .to_owned();
                    if !project.loadout.contains(&id) {
                        project.loadout.push(id);
                    }
                    appearance.equipment.remove(&slot);
                    adjustments.push(format!(
                        "{}: moved the former weapon appearance into the loadout as {}.",
                        appearance.name,
                        entry["record"].as_str().unwrap_or("weapon")
                    ));
                } else {
                    adjustments.push(format!("{}: could not map old weapon selection {selection} to an item record; retained it for correction.",appearance.name));
                }
            }
        }
        let validation = self.validate(&project);
        Ok(LoadedProject {
            project,
            adjustments,
            validation,
        })
    }

    pub fn validate(&self, project: &Project) -> Validation {
        let mut errors = Vec::new();
        let mut warnings = Vec::new();
        if let Err(error) = project.check() {
            errors.push(error.to_string());
        }
        match &project.catalogue {
            Some(pin) if *pin != self.pin => {
                warnings.push("Catalogue release changed; validation uses the current native selections.".into())
            }
            None => warnings.push("Project has no catalogue reference; validation uses the current native selections.".into()),
            _ => {}
        }
        for id in &project.loadout {
            if !self.entries.iter().any(|e| {
                e["kind"] == "loadout"
                    && e["id"] == *id
                    && e["record"]
                        .as_str()
                        .is_some_and(|r| r.starts_with("Items."))
            }) {
                errors.push(format!("unknown or unnamed loadout weapon: {id}"));
            }
        }
        if let Ok(template) = templates::get(&project.template) {
            if template.head_mode == "transfer" {
                for appearance in &project.appearances {
                    let mut selected = project.clone();
                    selected.active_appearance.clone_from(&appearance.id);
                    if let Err(error) = crate::frames::recipe(&selected, template) {
                        errors.push(format!("{}: {error}", appearance.name));
                    }
                    if let Err(error) = crate::frames::outfit_recipe(&selected, template) {
                        errors.push(format!("{}: {error}", appearance.name));
                    }
                }
            }
            for appearance in &project.appearances {
                for control in appearance
                    .choices
                    .keys()
                    .filter_map(|id| self.controls.get(id))
                {
                    if creator_frame(&control["source"])
                        .is_some_and(|frame| frame != template.frame)
                    {
                        errors.push(format!(
                            "{} uses a choice from the other creator frame",
                            appearance.name
                        ));
                    }
                }
                if !appearance.equipment.is_empty() {
                    for id in appearance.equipment.values() {
                        let entry = self
                            .entries
                            .iter()
                            .find(|entry| entry["kind"] == "equipment" && entry["id"] == *id);
                        let supported = entry
                            .and_then(|entry| entry["templates"].as_array())
                            .map_or(
                                !matches!(
                                    template.id,
                                    "female_big" | "male_big" | "female_fat" | "male_fat"
                                ),
                                |allowed| allowed.iter().any(|id| id == template.id),
                            );
                        if !supported {
                            errors.push(format!(
                                "{}: garment does not target {}: {id}",
                                appearance.name, template.id
                            ));
                        }
                        if self
                            .selections
                            .get(id)
                            .and_then(selection_frame)
                            .is_some_and(|frame| frame != template.frame)
                        {
                            errors.push(format!("{} uses equipment with an explicit condition for the other creator frame: {id}", appearance.name));
                        }
                    }
                }
            }
        }
        for appearance in &project.appearances {
            let mut selected = project.clone();
            selected.active_appearance.clone_from(&appearance.id);
            let selected_controls: Vec<_> = appearance
                .choices
                .keys()
                .filter_map(|id| self.controls.get(id))
                .collect();
            for (role, present) in [
                (
                    "head",
                    selected_controls
                        .iter()
                        .any(|c| c["native"]["ui_slot"] == "skin_type"),
                ),
                (
                    "body",
                    selected_controls
                        .iter()
                        .any(|c| c["native"]["ui_slot"] == "body_color"),
                ),
                (
                    "lower legs and feet",
                    templates::get(&project.template).is_ok_and(|t| t.frame == "male")
                        || selected_controls.iter().any(|c| {
                            c["native"]["ui_slot"] == "flat_feet"
                                || c["native"]["ui_slot"] == "lifted_feet"
                        }),
                ),
                (
                    "arms",
                    selected_controls.iter().any(|c| {
                        c["native"]["domain"] == "arms"
                            && c["native"]["name"]
                                .as_str()
                                .is_some_and(|n| n.starts_with("h_default_arms_colors"))
                    }),
                ),
            ] {
                if !present {
                    errors.push(format!(
                        "{} is missing its required {role} baseline",
                        appearance.name
                    ));
                }
            }
            match self.selected(&selected) {
                Ok(selections) => {
                    for selection in selections {
                        if selection["resolution"] != "resolved" {
                            errors.push(format!("unresolved selection: {}", selection["id"]));
                        }
                    }
                }
                Err(error) => errors.push(error.to_string()),
            }
        }
        Validation {
            ok: errors.is_empty(),
            errors,
            warnings,
        }
    }
}
