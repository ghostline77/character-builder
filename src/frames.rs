//! Reviewed native NPC body and head recipes.
use crate::{project::Project, templates::Template};
use anyhow::{Context, Result, ensure};
use serde_json::Value;
fn data(file: &str) -> Result<Value> {
    crate::templates::data(&format!("frames/{file}"))
}
pub fn default_head(frame: &str) -> &'static str {
    match frame {
        "male_big" => "head_h0_000_mb_c__basehead",
        "female_big" => "rhino",
        "male_fat" => "vendor",
        "female_fat" => "tenant",
        "male_average_npc" => "head_h0_003_ma_a__young",
        _ => "head_h0_000_wa__basehead",
    }
}
pub fn recipe(project: &Project, template: Template) -> Result<Value> {
    let library = data(&format!("npc-{}.components.json", template.id))?;
    let head = project
        .appearance()
        .native_head
        .as_deref()
        .unwrap_or_else(|| default_head(template.id));
    let module = &library["modules"]["heads"][head];
    ensure!(
        module.is_object(),
        "unknown native head {head} for {}",
        template.id
    );
    ensure!(
        module["body_family"]
            == library["modules"]["heads"][default_head(template.id)]["body_family"],
        "native head belongs to a different body family"
    );
    let provenance = data("npc-frames.provenance.json")?;
    let owner = provenance["families"]
        .as_array()
        .context("frame provenance absent")?
        .iter()
        .find(|v| v["frame"] == template.id)
        .context("frame provenance missing")?;
    let appearance = owner["bundles"][head]
        .as_str()
        .context("native head appearance absent")?;
    let family = module["body_family"]
        .as_str()
        .context("native body family absent")?;
    let body = data(&format!("npc-{family}.components.json"))?;
    Ok(
        serde_json::json!({"head":head,"module":module,"body_resource":body["source_depot"],"head_mesh":owner["heads"][head],"resource":owner["appearance_resource"],"appearance":appearance}),
    )
}
pub fn options(template: Template) -> Result<Value> {
    if template.head_mode != "transfer" {
        return Ok(Value::Null);
    }
    let library = data(&format!("npc-{}.components.json", template.id))?;
    Ok(Value::Array(
        library["modules"]["heads"]
            .as_object()
            .context("head modules absent")?
            .iter()
            .filter(|(_,module)|module["body_family"]==library["modules"]["heads"][default_head(template.id)]["body_family"])
            .map(|(id,_)| serde_json::json!({"id":id,"label":id.replace('_'," "),"default":id==default_head(template.id)}))
            .collect(),
    ))
}

/// Reviewed native wardrobe bundles, with their authored anatomical family.
pub fn outfits(template: Template) -> Result<Vec<Value>> {
    if template.head_mode != "transfer" {
        return Ok(Vec::new());
    }
    let library = data(&format!("npc-{}.components.json", template.id))?;
    let provenance = data("npc-frames.provenance.json")?;
    let owner = provenance["families"]
        .as_array()
        .context("frame provenance absent")?
        .iter()
        .find(|v| v["frame"] == template.id)
        .context("frame provenance missing")?;
    let family = &library["modules"]["heads"][default_head(template.id)]["body_family"];
    let mut outfits = Vec::new();
    for (id, module) in library["modules"]["heads"]
        .as_object()
        .context("head modules absent")?
    {
        if module["body_family"] != *family || module["outfit"].as_array().is_none_or(Vec::is_empty)
        {
            continue;
        }
        let appearance = owner["bundles"][id]
            .as_str()
            .context("outfit appearance absent")?;
        outfits.push(serde_json::json!({"id":format!("npc-outfit:{}:{id}",template.id),"native_outfit":id,"label":appearance.replace('_'," "),"category":"npc_outfit","kind":"npc_clothing","frame":template.frame,"template":template.id,"body_family":family,"rig":template.rig,"resource":{"Path":owner["appearance_resource"]},"appearance":appearance,"components":module["outfit"],"resolution":"resolved","media":[]}));
    }
    Ok(outfits)
}

pub fn outfit_recipe(project: &Project, template: Template) -> Result<Option<Value>> {
    let Some(id) = &project.appearance().native_outfit else {
        return Ok(None);
    };
    ensure!(
        template.head_mode == "transfer",
        "native NPC outfits require an NPC body template"
    );
    let mut selected = project.clone();
    selected
        .appearances
        .iter_mut()
        .find(|a| a.id == selected.active_appearance)
        .context("active appearance missing")?
        .native_head = Some(id.clone());
    let outfit = recipe(&selected, template)?;
    ensure!(
        outfit["module"]["outfit"]
            .as_array()
            .is_some_and(|v| !v.is_empty()),
        "native outfit has no components"
    );
    ensure!(
        outfit["module"]["body_family"] == recipe(project, template)?["module"]["body_family"],
        "outfit belongs to a different anatomical family"
    );
    Ok(Some(outfit))
}
