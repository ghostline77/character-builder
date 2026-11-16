//! Character component selection shared by previews and exported appearances.
use crate::{catalogue::Catalogue, project::Project, templates};
use anyhow::{Context, Result, ensure};
use ghostline_red::assets::{Assembly, AssemblyComponent, ResourceKey};
use ghostline_renderer::GameSource;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap};

pub struct Plan {
    pub native: Assembly,
    pub root_names: BTreeSet<String>,
    pub tags: BTreeSet<String>,
    pub transfers: HashMap<ResourceKey, ResourceKey>,
    pub head_mesh: Option<ResourceKey>,
    pub material_inventory: Vec<Value>,
    pub modified_meshes: HashMap<ResourceKey, Value>,
    pub source_diagnostics: Vec<Value>,
}

fn unwrap(value: &Value) -> &Value {
    value.get("Data").unwrap_or(value)
}
fn text(value: &Value) -> Option<&str> {
    value.as_str().or_else(|| value["$value"].as_str())
}

pub(crate) fn definition(
    source: &GameSource,
    resource: &ResourceKey,
    appearance: Option<&str>,
) -> Result<Value> {
    let document = source.resources().document(resource)?;
    let root = &document["Data"]["RootChunk"];
    if root["appearances"].as_array().is_none_or(Vec::is_empty) {
        return Ok(root.clone());
    }
    let selected = root["appearances"]
        .as_array()
        .context("appearance array absent")?
        .iter()
        .map(unwrap)
        .find(|a| text(&a["name"]) == appearance)
        .context("selected native appearance absent")?;
    if root["$type"] == "entEntityTemplate" {
        let resource = ResourceKey::from_value(&selected["appearanceResource"])?
            .context("appearance resource absent")?;
        return definition(source, &resource, text(&selected["appearanceName"]));
    }
    Ok(selected.clone())
}

pub(crate) fn merge(
    target: &mut Assembly,
    mut part: Assembly,
    index: usize,
) -> BTreeMap<String, String> {
    let mut renames = BTreeMap::new();
    for component in &part.components {
        if let Some(existing) = target.components.iter().find(|c| c.name == component.name)
            && existing.mesh.is_some()
            && component.mesh.is_some()
            && (existing.mesh != component.mesh
                || existing.mesh_appearance != component.mesh_appearance
                || existing.chunk_mask != component.chunk_mask)
        {
            renames.insert(
                component.name.clone(),
                format!("part{index}_{}", component.name),
            );
        }
    }
    for mut component in part.components.drain(..) {
        if let Some(name) = renames.get(&component.name) {
            component.name.clone_from(name);
        }
        for binding in [&mut component.parent, &mut component.control] {
            if let Some(name) = renames.get(&binding.component) {
                binding.component.clone_from(name);
            }
        }
        if let Some(existing) = target
            .components
            .iter_mut()
            .find(|c| c.name == component.name)
        {
            // External declarations refer to the actor's real component.
            if component.kind != "entExternalComponent" {
                *existing = component;
            }
        } else {
            target.components.push(component);
        }
    }
    target.rigs.extend(part.rigs);
    renames
}

pub fn resolve(project: &Project, catalogue: &Catalogue, source: &GameSource) -> Result<Plan> {
    resolve_with_fitting(project, catalogue, source, true)
}

pub fn resolve_with_fitting(
    project: &Project,
    catalogue: &Catalogue,
    source: &GameSource,
    fit: bool,
) -> Result<Plan> {
    let template = templates::get(&project.template)?;
    let mut root = templates::shell(template, "ent")?;
    root["Data"]["RootChunk"]["appearances"] = json!([]);
    let mut native = source.resources().assembly_document(
        &ResourceKey::parse(&format!("{}\\{}.ent", project.namespace, project.id)),
        &root,
        None,
    )?;
    let root_names = native.components.iter().map(|c| c.name.clone()).collect();
    let mut tags = BTreeSet::new();
    let mut overrides = Vec::new();
    let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut transfers = HashMap::new();
    let mut head_mesh = None;
    let mut material_inventory = Vec::new();
    let mut modified_meshes = HashMap::new();
    let mut source_diagnostics = Vec::new();
    let npc = if template.head_mode == "transfer" {
        Some(crate::frames::recipe(project, template)?)
    } else {
        None
    };
    if let Some(recipe) = &npc {
        let mut body = source
            .resources()
            .assembly(
                &ResourceKey::parse(
                    recipe["body_resource"]
                        .as_str()
                        .context("native body entity absent")?,
                ),
                None,
            )
            .context("native body recipe")?;
        let skin_donor = body
            .components
            .iter()
            .find(|c| c.name.starts_with("t0_") && c.mesh.is_some())
            .and_then(|c| c.mesh.clone());
        if let Some(outfit_recipe) = crate::frames::outfit_recipe(project, template)? {
            let names = outfit_recipe["module"]["outfit"]
                .as_array()
                .context("outfit component list missing")?
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect();
            let outfit = source
                .resources()
                .assembly_components(
                    &ResourceKey::parse(
                        outfit_recipe["resource"]
                            .as_str()
                            .context("outfit resource missing")?,
                    ),
                    outfit_recipe["appearance"].as_str(),
                    &names,
                )
                .context("native NPC outfit")?;
            // The donor's body pieces contain its garment hiding masks. Keeping
            // the bare body as well would fill those holes and cause clipping.
            if outfit.components.iter().any(|c| {
                c.mesh.is_some()
                    && ["t0_", "a0_", "l0_"]
                        .iter()
                        .any(|prefix| c.name.starts_with(prefix))
            }) {
                body.components.retain(|c| c.mesh.is_none());
            }
            merge(&mut body, outfit, 0);
        }
        for component in &mut body.components {
            if component.parent.component == "Component" {
                component.parent.component = "root".into()
            }
            if component.control.component == "Component" {
                component.control.component = "deformations".into()
            }
        }
        let mut body_names: BTreeSet<_> = body
            .components
            .iter()
            .filter(|c| c.mesh.is_some())
            .map(|c| c.name.clone())
            .collect();
        merge(&mut native, body, 0);
        let resource = ResourceKey::parse(
            recipe["resource"]
                .as_str()
                .context("NPC head resource missing")?,
        );
        let names: BTreeSet<String> = recipe["module"]["head"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect();
        let mut head = source
            .resources()
            .assembly_components(&resource, recipe["appearance"].as_str(), &names)
            .with_context(|| format!("native head {}#{}", resource, recipe["appearance"]))?;
        if let Some(path) = recipe["head_mesh"].as_str() {
            let key = ResourceKey::parse(path);
            head_mesh = Some(key.clone());
            for c in &mut head.components {
                if c.mesh
                    .as_ref()
                    .is_some_and(|m| m.native_hash() == key.native_hash())
                {
                    c.mesh = Some(key.clone());
                }
            }
        }
        merge(&mut native, head, 0);
        if let Some(mesh) = &head_mesh
            && let Some(head) = native
                .components
                .iter()
                .find(|c| c.mesh.as_ref() == Some(mesh))
        {
            let skin = source
                .resources()
                .mesh_appearance_binding(mesh, &head.mesh_appearance)?
                .chunk_materials;
            let head_name = head.name.clone();
            let body_tone_data = if let Some(tone) = &project.appearance().skin_tone {
                let donor_mesh = skin_donor
                    .as_ref()
                    .context("native skin donor mesh absent")?;
                let document = source.resources().document(donor_mesh)?;
                let appearance = find_skin_appearance(&document, std::slice::from_ref(tone))
                    .context("body skin tone unavailable")?;
                skin_data(source, donor_mesh, &appearance)?
            } else {
                Value::Null
            };
            body_names.insert(head_name.clone());
            let skin = project
                .appearance()
                .skin_tone
                .as_ref()
                .map_or(skin, |tone| vec![tone.clone()]);
            for body in native
                .components
                .iter_mut()
                .filter(|c| body_names.contains(&c.name))
            {
                if let Some(mesh) = &body.mesh {
                    let document = source.resources().document(mesh)?;
                    material_inventory.push(json!({"component":body.name,"requested_skin":skin,"appearances":document["Data"]["RootChunk"]["appearances"],"entries":document["Data"]["RootChunk"]["materialEntries"],"external":document["Data"]["RootChunk"]["externalMaterials"]}));
                    let appearance = document["Data"]["RootChunk"]["appearances"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(unwrap)
                        .find(|a| {
                            a["chunkMaterials"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .filter_map(text)
                                .any(|material| skin.iter().any(|s| s == material))
                        });
                    if let Some(appearance) = appearance
                        && let Some(name) = text(&appearance["name"])
                    {
                        body.mesh_appearance = name.into();
                    } else if project.appearance().skin_tone.is_some()
                        && (body.name == head_name
                            || ["t0_", "a0_", "l0_"]
                                .iter()
                                .any(|prefix| body.name.starts_with(prefix)))
                    {
                        let palette = if body.name == head_name {
                            let mut reference = project.clone();
                            reference
                                .appearances
                                .iter_mut()
                                .for_each(|a| a.native_head = None);
                            let reference_template =
                                templates::get(if template.frame == "female" {
                                    "female_average_npc"
                                } else {
                                    "male_average_npc"
                                })?;
                            let recipe = crate::frames::recipe(&reference, reference_template)?;
                            let reference_mesh = ResourceKey::parse(
                                recipe["head_mesh"]
                                    .as_str()
                                    .context("face palette mesh missing")?,
                            );
                            let reference_document =
                                source.resources().document(&reference_mesh)?;
                            let appearance = find_skin_appearance(&reference_document, &skin)
                                .context("face palette tone missing")?;
                            skin_data(source, &reference_mesh, &appearance)?
                        } else {
                            body_tone_data.clone()
                        };
                        let edited = recolour_skin(
                            source,
                            mesh,
                            &body.mesh_appearance,
                            &document,
                            &palette,
                        )?;
                        modified_meshes.insert(mesh.clone(), edited);
                    }
                }
            }
        }
    }
    let mut selections = catalogue.selected(project)?;
    if template.head_mode == "native"
        && let Some(control) = catalogue.controls.values().find(|c| {
            c["native"]["name"] == "tpp_head_face_rig"
                && crate::catalogue::creator_frame(&c["source"]) == Some(template.frame)
        })
        && let Some(selection) = catalogue
            .choices
            .values()
            .filter(|c| c["control"] == control["id"])
            .find_map(|c| {
                c["selection"]
                    .as_str()
                    .and_then(|id| catalogue.selections.get(id))
            })
    {
        selections.push(selection);
    }
    for (index, selection) in selections.iter().enumerate() {
        let resource = serde_json::from_value(selection["resource"].clone())?;
        let appearance = selection["appearance"].as_str();
        // A garment can carry its own deformation rig. Keep the owning native
        // component and its dependencies, including for alternate colours whose
        // ownership evidence is stored on another appearance of the same mesh.
        let owner = (selection["context"]["geometry_kind"] == "mesh")
            .then(|| {
                std::iter::once(*selection)
                    .chain(
                        catalogue
                            .selections
                            .values()
                            .filter(|candidate| candidate["resource"] == selection["resource"]),
                    )
                    .find_map(|candidate| {
                        candidate["context"]["clothing_owners"].as_array()?.first()
                    })
            })
            .flatten();
        let mut part = if let Some(owner) = owner {
            let owner_resource = serde_json::from_value(owner["resource"].clone())?;
            let names = [owner["component"]
                .as_str()
                .context("garment owner component absent")?
                .to_owned()]
            .into_iter()
            .collect();
            let mut part = source.resources().assembly_components(
                &owner_resource,
                owner["appearance"].as_str(),
                &names,
            )?;
            for component in &mut part.components {
                if component.mesh.as_ref() == Some(&resource) {
                    component.mesh_appearance =
                        appearance.context("garment appearance absent")?.into();
                }
                if component.parent.component == "Component" {
                    component.parent.component = "root".into();
                }
                if component.control.component == "Component" {
                    component.control.component = "deformations".into();
                }
            }
            part
        } else if selection["context"]["geometry_kind"] == "mesh" {
            let path = match &resource {
                ResourceKey::Path(path) => path.as_str(),
                _ => anyhow::bail!("direct clothing mesh requires a named depot path"),
            };
            let appearance = appearance.context("clothing mesh appearance missing")?;
            let name = format!("wardrobe_{}", resource.native_hash());
            let cname = |value: &str| json!({"$type":"CName","$storage":"string","$value":value});
            let document = json!({"Data":{"RootChunk":{"$type":"entEntityTemplate","components":[{"HandleId":"0","Data":{
                "$type":"entSkinnedMeshComponent","name":cname(&name),"isEnabled":1,"chunkMask":u64::MAX.to_string(),
                "mesh":{"DepotPath":{"$type":"ResourcePath","$storage":"string","$value":path},"Flags":"Default"},"meshAppearance":cname(appearance),
                "parentTransform":{"HandleId":"1","Data":{"$type":"entHardTransformBinding","bindName":cname("root"),"slotName":cname("None")}},
                "skinning":{"HandleId":"2","Data":{"$type":"entSkinningBinding","bindName":cname("deformations")}}
            }}]}}});
            source
                .resources()
                .assembly_document(&resource, &document, None)?
        } else {
            source.resources().assembly(&resource, appearance)?
        };
        // Standalone garment entities declare their owning actor externally.
        // Attach that declaration to the assembled character's root/palette.
        if selection["context"]["clothing"] == true {
            for component in &mut part.components {
                if component.parent.component == "Component" {
                    component.parent.component = "root".into();
                }
                if component.control.component == "Component" {
                    component.control.component = "deformations".into();
                }
            }
        }
        if npc.is_none()
            && project
                .appearance()
                .choices
                .iter()
                .any(|(control, choice)| {
                    catalogue
                        .controls
                        .get(control)
                        .is_some_and(|c| c["native"]["ui_slot"] == "skin_type")
                        && catalogue
                            .choices
                            .get(choice)
                            .is_some_and(|c| c["selection"] == selection["id"])
                })
        {
            let mut candidates = Vec::new();
            for mesh in part.components.iter().filter_map(|c| c.mesh.as_ref()) {
                let geometry = source.load_geometry(mesh)?;
                if !geometry.morph_targets().is_empty() {
                    candidates.push((
                        geometry
                            .chunks()
                            .iter()
                            .map(|c| c.positions.len())
                            .sum::<usize>(),
                        mesh.clone(),
                    ));
                }
            }
            head_mesh = candidates
                .into_iter()
                .max_by_key(|(count, _)| *count)
                .map(|(_, mesh)| mesh);
        }
        if npc.is_some() {
            let slots: Vec<_> = project
                .appearance()
                .choices
                .iter()
                .filter_map(|(control, id)| {
                    let choice = catalogue.choices.get(id)?;
                    (choice["selection"] == selection["id"])
                        .then(|| catalogue.controls.get(control))
                        .flatten()
                })
                .collect();
            if slots.iter().any(|c| {
                c["native"]["domain"] == "arms"
                    || matches!(
                        c["native"]["ui_slot"].as_str(),
                        Some(
                            "body_color"
                                | "flat_feet"
                                | "lifted_feet"
                                | "skin_type"
                                | "eyes_color"
                                | "eyebrows_color"
                                | "eyelash_color"
                                | "teeth"
                        )
                    )
            }) {
                let role = slots
                    .iter()
                    .find_map(|c| match c["native"]["ui_slot"].as_str() {
                        Some("skin_type") => Some("head"),
                        Some("eyes_color") => Some("eyes"),
                        Some("eyebrows_color") => Some("brows"),
                        Some("eyelash_color") => Some("lashes"),
                        Some("teeth") => Some("teeth"),
                        _ => None,
                    });
                if let Some(role) = role {
                    let mut candidates = Vec::new();
                    for mesh in part.components.iter().filter_map(|c| c.mesh.as_ref()) {
                        let geometry = source.load_geometry(mesh)?;
                        if !geometry.morph_targets().is_empty() {
                            candidates.push((
                                geometry
                                    .chunks()
                                    .iter()
                                    .map(|c| c.positions.len())
                                    .sum::<usize>(),
                                mesh,
                            ));
                        }
                    }
                    if let Some((_, morph)) = candidates.into_iter().max_by_key(|(count, _)| *count)
                    {
                        for target in &native.components {
                            if let Some(mesh) = &target.mesh {
                                let target_role =
                                    mesh_role(mesh).or_else(|| component_role(&target.name));
                                if target_role == Some(role) {
                                    transfers.insert(mesh.clone(), morph.clone());
                                }
                            }
                        }
                    }
                }
                continue;
            }
        }
        let names: Vec<_> = part
            .components
            .iter()
            .filter(|c| c.mesh.is_some())
            .map(|c| c.name.clone())
            .collect();
        let definition = if selection["context"]["geometry_kind"] == "mesh" {
            Value::Null
        } else {
            definition(source, &resource, appearance)?
        };
        for tag in definition["visualTags"]["tags"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(text)
        {
            tags.insert(tag.to_owned());
        }
        for entry in definition["partsOverrides"]
            .as_array()
            .into_iter()
            .flatten()
        {
            overrides.extend(
                unwrap(entry)["componentsOverrides"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    // The native resolver already applies overrides to this
                    // part's own components. Reapplying them to another part
                    // with the same component name intersects independent
                    // eye/lash masks and can hide the complete eye mesh.
                    .filter(|entry| {
                        !part.components.iter().any(|component| {
                            component.kind != "entExternalComponent"
                                && Some(component.name.as_str())
                                    == text(&unwrap(entry)["componentName"])
                        })
                    })
                    .cloned(),
            );
        }
        let renames = merge(&mut native, part, index);
        groups.insert(
            selection["id"]
                .as_str()
                .context("selection identity absent")?
                .into(),
            names
                .into_iter()
                .map(|name| renames.get(&name).cloned().unwrap_or(name))
                .collect(),
        );
    }
    for (index, part) in project.appearance().native_parts.iter().enumerate() {
        tags.extend(part.visual_tags.iter().cloned());
        merge(
            &mut native,
            crate::parts::assemble(part, source)?,
            selections.len() + index,
        );
    }
    for (path, value) in &project.appearance().mesh_overrides {
        let mesh = ResourceKey::parse(path);
        let replacement = value.resource.as_deref().map(ResourceKey::parse);
        if let Some(appearance) = &value.appearance {
            source
                .resources()
                .mesh_appearance_binding(replacement.as_ref().unwrap_or(&mesh), appearance)?;
        }
        let mut found = false;
        for component in native.components.iter_mut().filter(|c| {
            c.mesh
                .as_ref()
                .is_some_and(|key| key.native_hash() == mesh.native_hash())
        }) {
            found = true;
            if let Some(mesh) = &replacement {
                component.mesh = Some(mesh.clone());
            }
            if let Some(appearance) = &value.appearance {
                component.mesh_appearance.clone_from(appearance);
            }
            if let Some(mask) = &value.chunk_mask {
                component.chunk_mask = mask.parse().context("native mesh chunk mask")?;
            }
        }
        ensure!(
            found,
            "mesh override {path} does not belong to the selected appearance"
        );
    }
    for entry in overrides.iter().map(unwrap) {
        if let Some(component) = native
            .components
            .iter_mut()
            .find(|c| Some(c.name.as_str()) == text(&entry["componentName"]))
        {
            if let Some(mask) = entry["chunkMask"]
                .as_u64()
                .or_else(|| entry["chunkMask"].as_str()?.parse().ok())
            {
                component.chunk_mask &= mask;
            }
            if let Some(enabled) = entry["isEnabled"]
                .as_bool()
                .or_else(|| entry["isEnabled"].as_u64().map(|v| v != 0))
            {
                component.enabled &= enabled;
            }
        }
    }
    // These are the documented player body chunks, not assumptions about NPC meshes.
    for (control_id, choice_id) in &project.appearance().choices {
        let Some(control) = catalogue.controls.get(control_id) else {
            continue;
        };
        let Some(selection) = catalogue
            .choices
            .get(choice_id)
            .and_then(|c| c["selection"].as_str())
        else {
            continue;
        };
        let Some(names) = groups.get(selection) else {
            continue;
        };
        let slot = control["native"]["ui_slot"].as_str().unwrap_or("");
        let arm = control["native"]["domain"] == "arms";
        for component in native
            .components
            .iter_mut()
            .filter(|c| names.contains(&c.name))
        {
            if (slot == "hair_color" && tags.contains("hide_Hair") && !tags.contains("force_Hair"))
                || (slot == "skin_type" && tags.contains("hide_Head"))
                || (arm && tags.contains("hide_Arms"))
                || (matches!(slot, "genitals" | "underpants") && tags.contains("hide_Genitals"))
            {
                component.enabled = false;
            }
            let mut hidden = 0u64;
            for (tag, bit) in [
                ("hide_CollarBone", 1),
                ("hide_UpperAbdomen", 2),
                ("hide_LowerAbdomen", 3),
                ("hide_Thighs", 4),
                ("hide_Calves", 5),
                ("hide_Ankles", 6),
                ("hide_Feet", 7),
            ] {
                if tags.contains(tag) {
                    hidden |= 1 << bit;
                }
            }
            if tags.contains("hide_Torso") {
                hidden |= 7;
            }
            if tags.contains("hide_Legs") {
                hidden |= 0xf0;
            }
            if slot == "body_color" {
                component.chunk_mask &= !hidden;
            }
            if matches!(slot, "flat_feet" | "lifted_feet") {
                component.chunk_mask &= !(hidden >> 5);
            }
        }
    }
    for (slot, tag) in [
        ("Clo_Head", "hide_H1"),
        ("Clo_Face", "hide_F1"),
        ("Clo_InnerChest", "hide_T1"),
        ("Clo_OuterChest", "hide_T2"),
        ("Clo_Legs", "hide_L1"),
        ("Clo_Feet", "hide_S1"),
    ] {
        if tags.contains(tag)
            && let Some(selection) = project.appearance().equipment.get(slot)
            && let Some(names) = groups.get(selection)
        {
            for component in native
                .components
                .iter_mut()
                .filter(|c| names.contains(&c.name))
            {
                component.enabled = false;
            }
        }
    }
    ensure!(
        native
            .components
            .iter()
            .any(|c| c.enabled && c.mesh.is_some() && c.chunk_mask != 0),
        "character has no enabled geometry"
    );
    // Freeze native default aliases to authored names for both rendering and
    // export material readback, which requires an exact mesh appearance.
    for component in native
        .components
        .iter_mut()
        .filter(|c| c.enabled && c.chunk_mask != 0)
    {
        if let Some(mesh) = &component.mesh {
            component.mesh_appearance = source
                .resources()
                .mesh_appearance_binding(mesh, &component.mesh_appearance)?
                .appearance;
        }
    }
    // HQ body meshes need the deformation palette, while their transform parent
    // remains the locomotion root. Choose only a palette covering every bone.
    for index in 0..native.components.len() {
        let component = &native.components[index];
        let Some(mesh) = &component.mesh else {
            continue;
        };
        if !component.enabled
            || component.chunk_mask == 0
            || component.kind == "entAppearanceProxyMeshComponent"
        {
            continue;
        }
        let geometry = source.load_geometry(mesh)?;
        let Some(skin) = geometry.skin() else {
            continue;
        };
        let used: BTreeSet<usize> = geometry
            .chunks()
            .iter()
            .flat_map(|chunk| &chunk.skin)
            .flat_map(|vertex| {
                vertex
                    .joints
                    .iter()
                    .zip(vertex.weights)
                    .filter(|(_, weight)| *weight > 0.)
                    .map(|(&joint, _)| usize::from(joint))
            })
            .collect();
        let covers = |name: &str| {
            native
                .components
                .iter()
                .find(|c| c.name == name)
                .and_then(|c| c.rig.as_ref())
                .and_then(|r| native.rigs.get(r))
                .is_some_and(|rig| {
                    used.iter().all(|&index| {
                        skin.names
                            .get(index)
                            .is_some_and(|name| rig.bones.iter().any(|b| b.name == *name))
                    })
                })
        };
        if covers(&component.control.component) {
            continue;
        }
        let binding = [
            component.parent.component.as_str(),
            "deformations",
            "face_rig",
            "root",
        ]
        .into_iter()
        .find(|name| covers(name))
        .map(str::to_owned);
        if let Some(binding) = binding {
            native.components[index].control.component = binding.clone();
            let mut data = native.components[index].template.as_ref().clone();
            data["skinning"] = json!({"HandleId":"character_skin_binding","Data":{"$type":"entSkinningBinding","bindName":{"$type":"CName","$storage":"string","$value":binding}}});
            native.components[index].template = std::sync::Arc::new(data);
        }
    }
    for component in &mut native.components {
        if component.kind != "entAnimationSetupExtensionComponent" {
            continue;
        }
        let mut data = component.template.as_ref().clone();
        if let Some(groups) = data["animations"].as_object_mut() {
            for (group, entries) in groups {
                let Some(entries) = entries.as_array_mut() else {
                    continue;
                };
                let mut retained = Vec::new();
                for entry in entries.drain(..) {
                    if let Some(value) = entry.get("animSet")
                        && let Some(resource) = ResourceKey::from_value(value)?
                        && !source.resources().resource_exists(&resource)
                    {
                        source_diagnostics.push(json!({"component":component.name,"group":group,"resource":resource,"action":"omit unavailable optional animation setup clip"}));
                        continue;
                    }
                    retained.push(entry);
                }
                *entries = retained;
            }
        }
        component.template = std::sync::Arc::new(data);
    }
    if fit && project.refit_clothing {
        crate::fitting::fit_layers(
            project,
            catalogue,
            &mut native,
            &groups,
            source,
            &mut modified_meshes,
            &mut source_diagnostics,
        )?;
    }
    Ok(Plan {
        native,
        root_names,
        tags,
        transfers,
        head_mesh,
        material_inventory,
        modified_meshes,
        source_diagnostics,
    })
}

fn skin_data(source: &GameSource, mesh: &ResourceKey, appearance: &str) -> Result<Value> {
    let asset = source.load_shared_mesh_resource(mesh, appearance)?;
    let manifest = asset
        .material_provenance()?
        .context("skin material provenance missing")?;
    manifest["Materials"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|m| m["MaterialTemplate"] == r"base\materials\skin.mt")
        .map(|m| m["Data"].clone())
        .context("skin palette material missing")
}

fn recolour_skin(
    source: &GameSource,
    mesh: &ResourceKey,
    appearance: &str,
    document: &Value,
    palette: &Value,
) -> Result<Value> {
    let asset = source.load_shared_mesh_resource(mesh, appearance)?;
    let manifest = asset
        .material_provenance()?
        .context("skin material provenance missing")?;
    let mut color = palette["TintColor"].clone();
    color["$type"] = json!("Color");
    let parameters = vec![
        json!({"$type":"Color","TintColor":color}),
        json!({"$type":"Float","TintScale":palette["TintScale"]}),
        json!({"$type":"rRef:ITexture","Albedo":{"DepotPath":{"$type":"ResourcePath","$storage":"string","$value":palette["Albedo"]},"Flags":"Default"}}),
    ];
    let mut edited = document.clone();
    for material in manifest["Materials"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| m["MaterialTemplate"] == r"base\materials\skin.mt")
    {
        ghostline_red::material::override_material_parameters(
            &mut edited,
            material["Name"]
                .as_str()
                .context("skin material name missing")?,
            &parameters,
            source.schema(),
        )?;
    }
    Ok(edited)
}

fn find_skin_appearance(document: &Value, skin: &[String]) -> Option<String> {
    document["Data"]["RootChunk"]["appearances"]
        .as_array()?
        .iter()
        .map(unwrap)
        .find(|a| {
            a["chunkMaterials"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(text)
                .any(|m| skin.iter().any(|s| s == m))
        })
        .and_then(|a| text(&a["name"]))
        .map(str::to_owned)
}

pub fn mesh_role(key: &ResourceKey) -> Option<&'static str> {
    let ResourceKey::Path(path) = key else {
        return None;
    };
    let file = path.rsplit('\\').next()?;
    for (prefix, role) in [
        ("h0_", "head"),
        ("he_", "eyes"),
        ("heb_", "brows"),
        ("hel_", "lashes"),
        ("ht_", "teeth"),
        ("hx_", "head"),
    ] {
        if file.starts_with(prefix) {
            return Some(role);
        }
    }
    None
}

fn component_role(name: &str) -> Option<&'static str> {
    match name {
        "head" => Some("head"),
        "eyes" => Some("eyes"),
        "teeth" => Some("teeth"),
        "eyebrows" => Some("brows"),
        "eyelashes" => Some("lashes"),
        _ => mesh_role(&ResourceKey::parse(name)),
    }
}

pub fn component_document(component: &AssemblyComponent) -> Value {
    let mut data = component.template.as_ref().clone();
    data["name"] = json!({"$type":"CName","$storage":"string","$value":component.name});
    data["chunkMask"] = json!(component.chunk_mask.to_string());
    data["isEnabled"] = json!(u8::from(component.enabled));
    for (field, binding) in [
        ("parentTransform", &component.parent),
        ("controlBinding", &component.control),
        ("skinning", &component.control),
    ] {
        if data[field].get("Data").is_some() {
            data[field]["Data"]["bindName"] =
                json!({"$type":"CName","$storage":"string","$value":binding.component});
        }
    }
    data
}
