//! Repeatable clothing/rig/pose rendering and collision evidence.
use crate::{
    catalogue::Catalogue,
    native::{PreviewOptions, Session},
    poses,
    project::{self, Project},
    templates,
};
use anyhow::{Context, Result};
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
};

pub struct Options<'a> {
    pub inputs: &'a [PathBuf],
    pub samples_per_rig: usize,
    pub unfitted: bool,
    pub mask_components: &'a [String],
}
pub fn run(
    catalogue: &Catalogue,
    game: &Path,
    schema: Option<&Path>,
    out: &Path,
    options: Options<'_>,
) -> Result<()> {
    fs::create_dir_all(out)?;
    let diagnostic_chunk_masks = options
        .mask_components
        .iter()
        .map(|value| -> Result<_> {
            let (name, mask) = value
                .split_once('=')
                .context("diagnostic mask must be component=decimal-mask")?;
            Ok((
                name.to_owned(),
                mask.parse::<u64>()
                    .context("invalid diagnostic chunk mask")?,
            ))
        })
        .collect::<Result<std::collections::BTreeMap<_, _>>>()?;
    let mut projects = Vec::new();
    for input in options.inputs {
        let mut paths = if input.is_dir() {
            fs::read_dir(input)?
                .map(|entry| entry.map(|e| e.path()))
                .collect::<std::io::Result<Vec<_>>>()?
                .into_iter()
                .filter(|path| {
                    path.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|name| name.ends_with(".character.json"))
                })
                .collect::<Vec<_>>()
        } else {
            vec![input.clone()]
        };
        paths.sort();
        for path in paths {
            projects.push(catalogue.prepare_project(project::read(&path)?)?.project);
        }
    }
    for template in templates::TEMPLATES {
        for variant in 0..options.samples_per_rig {
            let mut project =
                Project::new(&format!("fit_{}_{}", template.id, variant), template.id)?;
            catalogue.defaults(&mut project)?;
            for category in ["Clo_InnerChest", "Clo_Legs", "Clo_Feet", "Clo_OuterChest"] {
                if variant == 0 && category == "Clo_OuterChest" {
                    continue;
                }
                let results = catalogue.search("", category, template.frame, template.id, 0, 120);
                let mut entries: Vec<_> = results["entries"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|e| e["resolution"] == "resolved" && e["kind"] == "equipment")
                    .filter(|e| !e["label"].as_str().unwrap_or("").contains("laces"))
                    .collect();
                entries.sort_by_key(|e| {
                    let id = e["id"].as_str().unwrap_or("");
                    (!id.contains(".ent#"), id)
                });
                if !entries.is_empty() {
                    let entry = entries[(variant * 7 + entries.len() / 3) % entries.len()];
                    project.appearances[0].equipment.insert(
                        category.into(),
                        entry["id"].as_str().context("garment id missing")?.into(),
                    );
                }
            }
            if !project.appearances[0].equipment.is_empty() {
                projects.push(project);
            }
        }
    }
    let mut identities = std::collections::BTreeSet::new();
    projects.retain(|project| identities.insert(project.id.clone()));
    // This command explicitly reviews fitting; ordinary projects remain opt-in.
    for project in &mut projects {
        project.refit_clothing = !options.unfitted;
    }
    fs::write(
        out.join("projects.json"),
        serde_json::to_vec_pretty(&projects)?,
    )?;
    let mut session = Session::open(game, schema)?;
    let mut report = Vec::new();
    for project in projects {
        project::write(
            &out.join(format!("{}.character.json", project.id)),
            &project,
        )?;
        for &pose in poses::CHECKS {
            for (view, yaw) in [("front", 0.), ("side", std::f32::consts::FRAC_PI_2)] {
                let directory = out.join(&project.id).join(pose.name()).join(view);
                let options = PreviewOptions {
                    pose,
                    yaw,
                    pitch: 0.,
                    size: 768,
                    diagnostic_unfitted: options.unfitted,
                    diagnostic_chunk_masks: diagnostic_chunk_masks.clone(),
                    ..PreviewOptions::default()
                };
                let result = session.preview(&project, catalogue, &directory, &options, || false);
                let row = match result {
                    Ok(value) => {
                        json!({"project":project.id,"template":project.template,"pose":pose,"view":view,"directory":directory,"result":value})
                    }
                    Err(error) => {
                        json!({"project":project.id,"template":project.template,"pose":pose,"view":view,"error":format!("{error:#}")})
                    }
                };
                println!(
                    "{} {} {}: {}",
                    project.id,
                    pose.name(),
                    view,
                    if row["error"].is_null() {
                        "rendered"
                    } else {
                        "FAILED"
                    }
                );
                report.push(row);
                fs::write(out.join("report.json"), serde_json::to_vec_pretty(&report)?)?;
            }
        }
    }
    anyhow::ensure!(
        report.iter().all(|row| row["error"].is_null()),
        "clothing pose review contains failures; see {}",
        out.join("report.json").display()
    );
    Ok(())
}
