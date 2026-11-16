//! Explicit native cyberware fit experiments, independent of clothing fitting.
use crate::{catalogue::Catalogue, poses::CheckPose, project::Project};
use anyhow::{Context, Result, ensure};
use ghostline_red::assets::{Assembly, ResourceKey, Rig, Transform};
use ghostline_renderer::{
    AssemblyOptions, Camera, GameSource, RenderOptions, Renderer, SceneAssembly,
};
use glam::{Mat4, Vec3};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
use std::{fs, path::Path};
pub struct Options<'a> {
    pub project: Option<&'a Path>,
    pub depot: &'a str,
    pub template: &'a str,
    pub entity: &'a str,
    pub fit: bool,
    pub overlays: &'a [std::path::PathBuf],
}

pub fn run(
    catalogue: &Catalogue,
    game: &Path,
    schema: Option<&Path>,
    out: &Path,
    options: Options<'_>,
) -> Result<()> {
    let Options {
        project: project_path,
        depot,
        template,
        entity,
        fit,
        overlays,
    } = options;
    ensure!(
        depot.starts_with("mod\\")
            && depot.split('\\').all(|p| !p.is_empty()
                && p != "."
                && p != ".."
                && p.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')),
        "cyberware depot must be a mod-owned path"
    );
    ensure!(
        !fit || template == "male_big",
        "mechanical fit review currently targets male_big"
    );
    fs::create_dir_all(out)?;
    let source = GameSource::open_with_overlays(game, schema, overlays)?;
    let project = if let Some(path) = project_path {
        let project = crate::project::read(path)?;
        ensure!(
            project.template == template,
            "cyberware target template differs from project"
        );
        catalogue.prepare_project(project)?.project
    } else {
        let mut project = Project::new("cyberware_fit_review", template)?;
        catalogue.defaults(&mut project)?;
        project
    };
    let plan = crate::assembly::resolve(&project, catalogue, &source)?;
    let mut cyberware = source
        .resources()
        .assembly(&ResourceKey::parse(entity), None)?;
    let animation = cyberware.components.iter().find_map(|c| {
        Some((
            c.name.clone(),
            c.template
                .pointer("/animations/gameplay/0/animSet/DepotPath/$value")?
                .as_str()?
                .to_owned(),
        ))
    });
    fs::write(
        out.join("cyberware-components.json"),
        serde_json::to_vec_pretty(&json!({
            "entity":entity,
            "components":cyberware.components.iter().map(|c|json!({"name":c.name,"kind":c.kind,"mesh":c.mesh,"appearance":c.mesh_appearance,"rig":c.rig,"parent":c.parent.component,"slot":c.parent.slot,"control":c.control.component,"template":c.template.as_ref()})).collect::<Vec<_>>(),
            "rigs":cyberware.rigs.iter().map(|(key,rig)|json!({"resource":key,"bones":rig.bones.iter().map(|b|json!({"name":b.name,"parent":b.parent,"translation":b.rest.translation})).collect::<Vec<_>>()})).collect::<Vec<_>>()
        }))?,
    )?;
    let mut modified = plan.modified_meshes;
    if fit {
        fit_armature(
            &source,
            &plan.native,
            &mut cyberware,
            &mut modified,
            game,
            schema,
            (out, depot),
        )?;
    }
    // A detached weapon entity has its own runtime animation controller. For
    // body-pose review, drive its anatomical anchors from the actual actor rig.
    for component in &mut cyberware.components {
        if component.rig.is_some() {
            component.control.component = "root".into();
        }
    }
    let mut native = plan.native;
    crate::assembly::merge(&mut native, cyberware, 999);
    let mut assembly =
        SceneAssembly::from_native(native, &AssemblyOptions::default(), |key, appearance| {
            if let Some(document) = modified.get(key) {
                source.load_material_override(key, appearance, document)
            } else {
                source.load_shared_mesh_resource(key, appearance)
            }
        })?;
    let renderer = pollster::block_on(Renderer::new())?;
    let rig = *assembly.rigs.get("root").context("body root rig absent")?;
    let skeleton = assembly.scene.skeleton(rig)?.clone();
    for pose in [CheckPose::Rest, CheckPose::Relaxed, CheckPose::Reach] {
        assembly.scene.set_pose(rig, &pose.sample(&skeleton)?)?;
        let directory = out.join(pose.name());
        fs::create_dir_all(&directory)?;
        fs::write(
            directory.join("assembly.json"),
            serde_json::to_vec_pretty(&assembly.inspection(&[])?)?,
        )?;
        let mut prepared = renderer.prepare_scene(&assembly.scene)?;
        for (view, yaw) in [
            ("front", 0_f32),
            ("hero", -0.65),
            ("side", std::f32::consts::FRAC_PI_2),
        ] {
            let target = glam::Vec3::new(0., 1.3, 0.);
            let eye = target + glam::Vec3::new(yaw.sin(), 0., -yaw.cos()) * 3.;
            renderer
                .render(
                    &mut prepared,
                    RenderOptions {
                        size: 1024,
                        camera: Camera::Orthographic {
                            eye: eye.to_array(),
                            target: target.to_array(),
                            up: [0., 1., 0.],
                            half_height: 1.0,
                            near: 0.001,
                            far: 20.,
                        },
                    },
                )?
                .image
                .save(directory.join(format!("{view}.png")))?;
            println!("{template} cyberware {} {view}: rendered", pose.name());
        }
    }
    if let Some((component, animation)) = animation {
        render_deployment(
            &source,
            &mut assembly,
            &renderer,
            &component,
            &animation,
            out,
        )?;
    }
    Ok(())
}

fn render_deployment(
    source: &GameSource,
    assembly: &mut SceneAssembly,
    renderer: &Renderer,
    component: &str,
    animation: &str,
    out: &Path,
) -> Result<()> {
    let root = *assembly.rigs.get("root").context("body rig absent")?;
    let child = *assembly
        .rigs
        .get(component)
        .context("mechanical rig absent")?;
    let body = assembly.scene.skeleton(root)?.clone();
    let body_pose = CheckPose::Relaxed.sample(&body)?;
    assembly.scene.set_pose(root, &body_pose)?;
    let body_models = body.model_matrices(&body_pose)?;
    let fitted = assembly.scene.skeleton(child)?.clone();
    let native_key = assembly
        .native
        .components
        .iter()
        .find(|c| c.name == component)
        .and_then(|c| c.rig.as_ref())
        .context("mechanical rig identity absent")?;
    let parents: Vec<_> = assembly.native.rigs[native_key]
        .bones
        .iter()
        .map(|b| b.parent)
        .collect();
    let mut reader = source.resources();
    let clip = reader.animation(&ResourceKey::parse(animation), "unequip")?;
    let clip_rig = reader.rig(clip.rig.as_ref().context("native animation rig absent")?)?;
    let reference: Vec<_> = clip_rig.bones.iter().map(|b| b.rest).collect();
    let basis = Mat4::from_cols_array(&[
        1., 0., 0., 0., 0., 0., -1., 0., 0., 1., 0., 0., 0., 0., 0., 1.,
    ]);
    let anchors: BTreeMap<_, _> = fitted
        .bone_names()
        .iter()
        .filter_map(|name| {
            let i = body.bone_names().iter().position(|n| n == name)?;
            Some((name.clone(), body_models[i]))
        })
        .collect();
    ensure!(
        assembly.scene.unlink_rig(child),
        "mechanical body driver was not linked"
    );
    let mut report = Vec::new();
    for (label, fraction) in [("deployed", 0.), ("retracting", 0.5), ("holstered", 1.)] {
        let sample = clip.sample(clip.duration * fraction, &reference, None)?;
        let mut pose = fitted.rest_pose().clone();
        for (i, name) in fitted.bone_names().iter().enumerate() {
            if let Some(j) = clip_rig.bones.iter().position(|b| &b.name == name) {
                let delta = basis
                    * reference[j].matrix()?.inverse()
                    * sample.local[j].matrix()?
                    * basis.transpose();
                pose.local[i] *= delta;
            }
        }
        let models = fitted.model_matrices(&pose)?;
        let models =
            ghostline_refitter::armature::rebase(fitted.bone_names(), &parents, &models, &anchors)?;
        for (i, local) in pose.local.iter_mut().enumerate() {
            *local = parents[i].map_or(models[i], |p| models[p].inverse() * models[i]);
        }
        assembly.scene.set_pose(child, &pose)?;
        let directory = out.join(label);
        fs::create_dir_all(&directory)?;
        let mut prepared = renderer.prepare_scene(&assembly.scene)?;
        for (view, yaw) in [
            ("front", 0_f32),
            ("hero", -0.65),
            ("side", std::f32::consts::FRAC_PI_2),
        ] {
            let target = Vec3::new(0., 1.3, 0.);
            let eye = target + Vec3::new(yaw.sin(), 0., -yaw.cos()) * 3.;
            renderer
                .render(
                    &mut prepared,
                    RenderOptions {
                        size: 1024,
                        camera: Camera::Orthographic {
                            eye: eye.to_array(),
                            target: target.to_array(),
                            up: [0., 1., 0.],
                            half_height: 1.,
                            near: 0.001,
                            far: 20.,
                        },
                    },
                )?
                .image
                .save(directory.join(format!("{view}.png")))?;
        }
        fs::write(
            directory.join("assembly.json"),
            serde_json::to_vec_pretty(&assembly.inspection(&[])?)?,
        )?;
        report.push(json!({"state":label,"clip":"unequip","animset":animation,"time":clip.duration*fraction,"source_rig":clip.rig,"mapped_joints":fitted.bone_names().iter().filter(|n|clip_rig.bones.iter().any(|b|&b.name==*n)).count(),"body_anchors":anchors.keys().collect::<Vec<_>>() }));
        println!("native mantis {label}: rendered");
    }
    fs::write(
        out.join("deployment-report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    Ok(())
}

fn local_transforms(models: &[Mat4], parents: &[Option<usize>]) -> Result<Vec<Transform>> {
    let basis = Mat4::from_cols_array(&[
        1., 0., 0., 0., 0., 0., -1., 0., 0., 1., 0., 0., 0., 0., 0., 1.,
    ]);
    models
        .iter()
        .enumerate()
        .map(|(i, &model)| {
            let local = parents[i].map_or(model, |p| models[p].inverse() * model);
            let red = basis.transpose() * local * basis;
            let (scale, rotation, translation) = red.to_scale_rotation_translation();
            ensure!(
                Mat4::from_scale_rotation_translation(scale, rotation, translation)
                    .abs_diff_eq(red, 1e-4),
                "mechanical rebase introduced shear"
            );
            Ok(Transform {
                translation: translation.to_array(),
                rotation: rotation.normalize().to_array(),
                scale: scale.to_array(),
            })
        })
        .collect()
}

fn fit_armature(
    source: &GameSource,
    body: &Assembly,
    gear: &mut Assembly,
    modified: &mut HashMap<ResourceKey, Value>,
    game: &Path,
    schema_path: Option<&Path>,
    destination: (&Path, &str),
) -> Result<()> {
    let (out, depot) = destination;
    ensure!(
        gear.rigs.len() == 1,
        "cyberware fit requires one independent mechanical rig"
    );
    let (key, original) = gear.rigs.iter().next().context("cyberware rig absent")?;
    let key = key.clone();
    let original = original.clone();
    let body_key = body
        .components
        .iter()
        .find(|c| c.name == "root")
        .and_then(|c| c.rig.as_ref())
        .context("body root resource absent")?;
    let target = ghostline_renderer::Skeleton::from_rig(&body.rigs[body_key])?;
    let target_models = target.model_matrices(target.rest_pose())?;
    let source_rig = ghostline_renderer::Skeleton::from_rig(&original)?;
    let source_models = source_rig.model_matrices(source_rig.rest_pose())?;
    let anchors: BTreeMap<_, _> = original
        .bones
        .iter()
        .filter_map(|bone| {
            let index = target
                .bone_names()
                .iter()
                .position(|name| name == &bone.name)?;
            let (_, rotation, translation) = target_models[index].to_scale_rotation_translation();
            Some((
                bone.name.clone(),
                Mat4::from_rotation_translation(rotation, translation),
            ))
        })
        .collect();
    ensure!(
        ["LeftForeArm", "RightForeArm"]
            .iter()
            .all(|name| anchors.contains_key(*name)),
        "cyberware rig lacks two anatomical forearm anchors"
    );
    let parents: Vec<_> = original.bones.iter().map(|b| b.parent).collect();
    let names: Vec<_> = original.bones.iter().map(|b| b.name.clone()).collect();
    let fitted = ghostline_refitter::armature::rebase(&names, &parents, &source_models, &anchors)?;
    let deltas: Vec<_> = fitted
        .iter()
        .zip(&source_models)
        .map(|(&to, &from)| to * from.inverse())
        .collect();
    let basis = Mat4::from_cols_array(&[
        1., 0., 0., 0., 0., 0., -1., 0., 0., 1., 0., 0., 0., 0., 0., 1.,
    ]);
    let reference: Vec<_> = original
        .model_rest()?
        .iter()
        .zip(&deltas)
        .map(|(&model, &delta)| delta * basis * model * basis.transpose())
        .collect();
    let reference = local_transforms(&reference, &parents)?;
    let bind = original
        .a_pose
        .as_ref()
        .map(|_| local_transforms(&fitted, &parents))
        .transpose()?;
    let mut reader = source.resources();
    let mut rig_document = reader.document(&key)?.as_ref().clone();
    Rig::replace_reference_document(&mut rig_document, &reference, bind.as_deref())?;
    let mut new_rig = Rig::decode(&rig_document)?;
    new_rig.source = Some(key.clone());
    gear.rigs.insert(key.clone(), new_rig);
    let mut documents = BTreeMap::new();
    documents.insert("mantis.rig".to_owned(), (key.clone(), rig_document));
    let meshes: BTreeMap<_, _> = gear
        .components
        .iter()
        .filter_map(|c| c.mesh.clone())
        .map(|key| (key.native_hash(), key))
        .collect();
    let mut diagnostics = Vec::new();
    for (mesh_index, mesh) in meshes.into_values().enumerate() {
        let mut document = reader.document(&mesh)?.as_ref().clone();
        let skin = ghostline_red::mesh::decode_skin(&document)?
            .context("cyberware mesh has no skinning")?;
        let maps: Vec<_> = skin
            .names
            .iter()
            .map(|name| {
                names
                    .iter()
                    .position(|n| n == name)
                    .map(|i| deltas[i])
                    .with_context(|| format!("unbound mechanical joint {name}"))
            })
            .collect::<Result<_>>()?;
        let mut chunks = ghostline_red::mesh::decode_geometry(&document, false)?;
        let mut moved = 0;
        let mut maximum = 0_f32;
        for chunk in &mut chunks {
            ensure!(
                chunk.positions.len() == chunk.skin.len(),
                "cyberware vertex skin stream absent"
            );
            for i in 0..chunk.positions.len() {
                let skin = &chunk.skin[i];
                let total: f32 = skin.weights.iter().sum();
                if total <= 1e-8 {
                    continue;
                }
                let mut transform = Mat4::ZERO;
                for (&joint, &weight) in skin
                    .joints
                    .iter()
                    .zip(&skin.weights)
                    .filter(|(_, w)| **w > 0.)
                {
                    transform += *maps
                        .get(usize::from(joint))
                        .context("cyberware joint index out of range")?
                        * (weight / total);
                }
                let before = Vec3::from_array(chunk.positions[i]);
                let after = transform.transform_point3(before);
                chunk.positions[i] = after.to_array();
                moved += usize::from(before.distance(after) > 1e-6);
                maximum = maximum.max(before.distance(after));
                let normal = transform
                    .inverse()
                    .transpose()
                    .transform_vector3(chunk.normals[i].into())
                    .normalize_or_zero();
                chunk.normals[i] = normal.to_array();
                if let Some(tangent) = chunk.tangents.get_mut(i) {
                    let t =
                        transform.transform_vector3(Vec3::new(tangent[0], tangent[1], tangent[2]));
                    let t = (t - normal * t.dot(normal)).normalize_or_zero();
                    *tangent = [t.x, t.y, t.z, tangent[3]];
                }
            }
        }
        ghostline_red::mesh::replace_geometry(&mut document, &chunks)?;
        let inverse: Vec<_> = skin
            .inverse_bind
            .iter()
            .zip(&maps)
            .map(|(&inverse, &delta)| inverse * delta.inverse())
            .collect();
        ghostline_red::mesh::replace_inverse_bind(&mut document, &inverse)?;
        modified.insert(mesh.clone(), document.clone());
        diagnostics.push(json!({"mesh":mesh,"moved_vertices":moved,"maximum_displacement_m":maximum,"chunks":chunks.len()}));
        documents.insert(format!("mantis_{mesh_index}.mesh"), (mesh, document));
    }
    let schema = ghostline_red::schema::RedSchema::load(Some(game), schema_path)?;
    let raw = out.join("source");
    let native = out.join("native").join(depot.replace('\\', "/"));
    fs::create_dir_all(&raw)?;
    fs::create_dir_all(&native)?;
    let replacements: BTreeMap<_, _> = documents
        .iter()
        .map(|(file, (key, _))| (key.to_string(), format!("{depot}\\{file}")))
        .collect();
    let entity_key = gear.source.clone();
    let mut entity = reader.document(&entity_key)?.as_ref().clone();
    replace_paths(&mut entity, &replacements);
    documents.insert("mantis.ent".into(), (entity_key, entity));
    for (file, (_, document)) in &documents {
        let json_path = raw.join(format!("{file}.json"));
        let binary = native.join(file);
        fs::write(&json_path, serde_json::to_vec_pretty(document)?)?;
        ghostline_red::writer::write(&json_path, &binary, &schema)?;
        let decoded = ghostline_red::codec::decode_wkit_with_red_schema(&binary, &schema)?;
        if file.ends_with(".rig") {
            let actual = Rig::decode(&decoded)?;
            let expected = Rig::decode(document)?;
            ensure!(
                actual.bones.len() == expected.bones.len()
                    && actual
                        .bones
                        .iter()
                        .zip(&expected.bones)
                        .all(|(a, b)| a.name == b.name && a.parent == b.parent),
                "exported rig hierarchy differs"
            );
            for (a, b) in actual.model_rest()?.iter().zip(expected.model_rest()?) {
                ensure!(a.abs_diff_eq(b, 1e-5), "exported rig reference differs");
            }
        }
        if file.ends_with(".mesh") {
            let expected = ghostline_red::mesh::decode_geometry(document, false)?;
            let actual = ghostline_red::mesh::decode_geometry(&decoded, false)?;
            ensure!(
                expected.len() == actual.len(),
                "exported mesh chunk count differs"
            );
            for (a, b) in expected.iter().zip(&actual) {
                ensure!(
                    a.indices == b.indices
                        && a.skin.len() == b.skin.len()
                        && a.skin
                            .iter()
                            .zip(&b.skin)
                            .all(|(a, b)| a.joints == b.joints && a.weights == b.weights)
                        && a.positions == b.positions
                        && a.tex_coords == b.tex_coords
                        && a.colors == b.colors,
                    "exported geometry or weights differ"
                );
            }
            let expected =
                ghostline_red::mesh::decode_skin(document)?.context("expected skin absent")?;
            let actual =
                ghostline_red::mesh::decode_skin(&decoded)?.context("exported skin absent")?;
            ensure!(
                expected.names == actual.names
                    && expected
                        .inverse_bind
                        .iter()
                        .zip(&actual.inverse_bind)
                        .all(|(a, b)| a.abs_diff_eq(*b, 1e-6)),
                "exported skin bindings differ"
            );
        }
    }
    fs::write(
        out.join("fit-report.json"),
        serde_json::to_vec_pretty(
            &json!({"source":gear.source,"target_rig":body_key,"anchors":anchors.keys().collect::<Vec<_>>(),"anchor_error_m":names.iter().enumerate().filter_map(|(i,n)|anchors.get(n).map(|m|fitted[i].w_axis.truncate().distance(m.w_axis.truncate()))).fold(0_f32,f32::max),"meshes":diagnostics,"native_files":documents.keys().collect::<Vec<_>>(),"automatic_clothing_refit":false,"in_game_verified":false}),
        )?,
    )?;
    Ok(())
}

fn replace_paths(value: &mut Value, replacements: &BTreeMap<String, String>) {
    match value {
        Value::String(s) => {
            if let Some(replacement) = replacements.get(s) {
                *s = replacement.clone();
            }
        }
        Value::Array(values) => {
            for value in values {
                replace_paths(value, replacements);
            }
        }
        Value::Object(values) => {
            for value in values.values_mut() {
                replace_paths(value, replacements);
            }
        }
        _ => {}
    }
}
