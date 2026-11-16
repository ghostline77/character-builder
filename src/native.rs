use std::{collections::BTreeMap, fs, path::Path};

use anyhow::{Context, Result, ensure};
use ghostline_red::assets::ResourceKey;
use ghostline_renderer::{
    AssemblyOptions, Camera, GameSource, PreparedScene, RenderOptions, Renderer,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    catalogue::Catalogue,
    project::{self, Project},
    templates,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PreviewOptions {
    /// Prepare an interactive scene instead of an offscreen camera image.
    #[serde(default)]
    pub browser_scene: bool,
    pub yaw: f32,
    pub pitch: f32,
    pub zoom: f32,
    pub size: u32,
    #[serde(default)]
    pub focus_head: bool,
    /// Diagnostic comparison only: replace the head surface with catalogue grey.
    #[serde(default)]
    pub diagnostic_neutral_head: bool,
    #[serde(default)]
    pub pose: crate::poses::CheckPose,
    #[serde(default)]
    pub diagnostic_unfitted: bool,
    /// Draw-only diagnostic masks, applied after native rig assembly.
    #[serde(default)]
    pub diagnostic_chunk_masks: std::collections::BTreeMap<String, u64>,
}

impl Default for PreviewOptions {
    fn default() -> Self {
        Self {
            browser_scene: false,
            yaw: -0.55,
            pitch: 0.12,
            zoom: 1.,
            size: 768,
            focus_head: false,
            diagnostic_neutral_head: false,
            pose: crate::poses::CheckPose::Rest,
            diagnostic_unfitted: false,
            diagnostic_chunk_masks: Default::default(),
        }
    }
}

pub struct Session {
    source: GameSource,
    renderer: Renderer,
    prepared: Option<PreparedScene>,
    nodes: BTreeMap<String, ghostline_renderer::NodeId>,
    assembly_key: String,
    inspection: Value,
    bounds: (glam::Vec3, f32),
    head_bounds: Option<(glam::Vec3, f32)>,
    _preparation: Option<tempfile::TempDir>,
    pose_rig: Option<(ghostline_renderer::RigId, ghostline_renderer::Skeleton)>,
}

impl Session {
    /// Renders exact native clips through the normal character assembly, with fitting disabled.
    pub fn render_animation_clips(
        &mut self,
        project: &Project,
        catalogue: &Catalogue,
        manifest: &Path,
        out: &Path,
    ) -> Result<()> {
        ensure!(
            !project.refit_clothing,
            "Animation previews do not run the refitter"
        );
        ensure!(!out.exists(), "Animation output must be new");
        #[derive(Deserialize)]
        struct Clip {
            label: String,
            animset: String,
            animation: String,
        }
        let clips: Vec<Clip> = serde_json::from_slice(&fs::read(manifest)?)?;
        ensure!(
            !clips.is_empty() && clips.len() <= 20,
            "Expected 1..20 clips"
        );
        let options = PreviewOptions {
            size: 640,
            diagnostic_unfitted: true,
            ..Default::default()
        };
        self.preview(project, catalogue, &out.join("assembly"), &options, || {
            false
        })?;
        let (rig, _) = self.pose_rig.as_ref().context("No body rig")?;
        let rig = *rig;
        let prepared = self.prepared.as_mut().context("No prepared scene")?;
        let mut reader = self.source.resources();
        let mut reports = Vec::new();
        for request in clips {
            ensure!(
                !request.label.is_empty()
                    && request
                        .label
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_'),
                "Invalid clip label"
            );
            println!("Preparing {}", request.label);
            let clip =
                reader.animation(&ResourceKey::parse(&request.animset), &request.animation)?;
            let duration = clip.duration.min(5.);
            let mut minimum = glam::Vec3::splat(f32::INFINITY);
            let mut maximum = glam::Vec3::splat(f32::NEG_INFINITY);
            for i in 0..=8 {
                prepared.apply_animation(
                    rig,
                    &clip,
                    duration * i as f32 / 8.,
                    None,
                    ghostline_renderer::RootMotion::Preserve,
                )?;
                for chunk in prepared.evaluated_geometry()? {
                    for point in chunk.positions {
                        let point = glam::Vec3::from(point);
                        minimum = minimum.min(point);
                        maximum = maximum.max(point);
                    }
                }
            }
            let center = (minimum + maximum) * 0.5;
            let radius = (maximum - minimum).length() * 0.5;
            ensure!(
                center.is_finite() && radius > 0.1 && radius < 10.,
                "Invalid animated bounds"
            );
            let camera = Camera::Orthographic {
                eye: (center + glam::Vec3::new(-0.65, 0.25, -1.).normalize() * radius * 4.)
                    .to_array(),
                target: center.to_array(),
                up: [0., 1., 0.],
                half_height: radius * 1.08,
                near: 0.01,
                far: radius * 12.,
            };
            let frames: Vec<_> = (0..(duration * 12.).ceil() as u32)
                .map(|i| ghostline_renderer::FrameRequest {
                    time: i as f32 / 12.,
                    render: RenderOptions { size: 640, camera },
                })
                .collect();
            let identity = serde_json::to_string(
                &json!({"game":self.source.content_identity(),"project":project,"clip":request.animation,"animset":request.animset,"refitter_invoked":false,"runner":include_str!("native.rs")}),
            )?;
            let report = self.renderer.render_sequence(
                prepared,
                &frames,
                &out.join(&request.label),
                &identity,
                &ghostline_renderer::Cancellation::default(),
                |scene, time| {
                    scene.apply_animation(
                        rig,
                        &clip,
                        time,
                        None,
                        ghostline_renderer::RootMotion::Preserve,
                    )?;
                    Ok(())
                },
            )?;
            reports.push(json!({"label":request.label,"animation":request.animation,"animset":request.animset,"source_duration":clip.duration,"report":report}));
            fs::write(
                out.join("report.json"),
                serde_json::to_vec_pretty(
                    &json!({"project":project.id,"refit_clothing":false,"refitter_invoked":false,"source":self.source.content_identity(),"clips":reports}),
                )?,
            )?;
            println!("Finished {}", request.label);
        }
        Ok(())
    }

    pub fn open(game: &Path, schema: Option<&Path>) -> Result<Self> {
        Ok(Self {
            source: GameSource::open(game, schema)?,
            renderer: pollster::block_on(Renderer::new())?,
            prepared: None,
            nodes: BTreeMap::new(),
            assembly_key: String::new(),
            inspection: Value::Null,
            bounds: (glam::Vec3::new(0., 0.95, 0.), 1.05),
            head_bounds: None,
            _preparation: None,
            pose_rig: None,
        })
    }

    pub fn preview(
        &mut self,
        project: &Project,
        catalogue: &Catalogue,
        output: &Path,
        options: &PreviewOptions,
        cancelled: impl Fn() -> bool,
    ) -> Result<Value> {
        project.check()?;
        let validation = catalogue.validate(project);
        ensure!(validation.ok, "{}", validation.errors.join("; "));
        ensure!(
            (128..=2048).contains(&options.size)
                && options.yaw.is_finite()
                && options.pitch.is_finite()
                && options.zoom.is_finite()
                && (0.2..=5.).contains(&options.zoom),
            "invalid preview camera"
        );
        let selections = catalogue.selected(project)?;
        ensure!(!selections.is_empty(), "character has no native selections");
        let key = serde_json::to_string(&(
            project.template.as_str(),
            &project.appearance().native_head,
            &project.appearance().skin_tone,
            &project.appearance().native_outfit,
            &project.appearance().mesh_overrides,
            &project.appearance().native_parts,
            options.diagnostic_neutral_head,
            options.diagnostic_unfitted,
            project.refit_clothing,
            &options.diagnostic_chunk_masks,
            selections.iter().map(|s| &s["id"]).collect::<Vec<_>>(),
        ))?;
        if key != self.assembly_key {
            ensure!(!cancelled(), "preview cancelled");
            let preparation = tempfile::Builder::new()
                .prefix("character-assembly-")
                .tempdir()?;
            let plan = crate::assembly::resolve_with_fitting(
                project,
                catalogue,
                &self.source,
                !options.diagnostic_unfitted,
            )?;
            fs::create_dir_all(output)?;
            fs::write(
                output.join("material-inventory.json"),
                serde_json::to_vec_pretty(&plan.material_inventory)?,
            )?;
            let head_names: std::collections::BTreeSet<_> = plan
                .native
                .components
                .iter()
                .filter(|c| c.mesh.is_some() && c.mesh == plan.head_mesh)
                .map(|c| c.name.clone())
                .collect();
            fs::create_dir_all(output)?;
            fs::write(
                output.join("component-plan.json"),
                serde_json::to_vec_pretty(
                    &json!({"components":plan.native.components.iter().map(|c|json!({"name":c.name,"mesh":c.mesh,"rig":c.rig,"parent":c.parent.component,"control":c.control.component,"enabled":c.enabled,"mask":c.chunk_mask.to_string()})).collect::<Vec<_>>(),"rigs":plan.native.rigs.iter().map(|(key,rig)|json!({"key":key,"bones":rig.bones.iter().map(|b|&b.name).collect::<Vec<_>>()})).collect::<Vec<_>>()}),
                )?,
            )?;
            let mut assembly = ghostline_renderer::SceneAssembly::from_native(
                plan.native,
                &AssemblyOptions::default(),
                |resource, appearance| {
                    let asset = if options.diagnostic_neutral_head
                        && plan.head_mesh.as_ref() == Some(resource)
                    {
                        std::sync::Arc::new(
                            self.source
                                .load_geometry(resource)?
                                .with_neutral_material()?,
                        )
                    } else if let Some(document) = plan.modified_meshes.get(resource) {
                        self.source
                            .load_material_override(resource, appearance, document)?
                    } else {
                        self.source
                            .load_shared_mesh_resource(resource, appearance)?
                    };
                    if let Some(morph_source) = plan.transfers.get(resource) {
                        let geometry = self.source.load_geometry(morph_source)?;
                        if !geometry.morph_targets().is_empty() {
                            let document = self.source.resources().document(resource)?;
                            let chunks = ghostline_red::mesh::decode_geometry(&document, true)?;
                            let neck = if crate::assembly::mesh_role(resource) == Some("head") {
                                ghostline_red::mesh::decode_skin(&document)?
                                    .as_ref()
                                    .and_then(|s| {
                                        s.names
                                            .iter()
                                            .position(|n| n == "Neck1")
                                            .map(|i| s.inverse_bind[i].inverse().w_axis.y)
                                    })
                            } else {
                                None
                            };
                            let targets = ghostline_red::morph_transfer::transfer(
                                geometry.chunks(),
                                geometry.morph_targets(),
                                &chunks,
                                neck,
                            )?;
                            return Ok(std::sync::Arc::new(asset.with_morph_targets(targets)));
                        }
                    }
                    Ok(asset)
                },
            )?;
            ensure!(!cancelled(), "preview cancelled");
            for (name, mask) in &options.diagnostic_chunk_masks {
                let node = *assembly
                    .components
                    .get(name)
                    .with_context(|| format!("diagnostic component absent: {name}"))?;
                assembly.scene.set_chunk_mask(node, *mask)?;
            }
            self.nodes = assembly
                .components
                .iter()
                .map(|(name, node)| (name.clone(), *node))
                .collect();
            self.inspection = assembly.inspection(&[])?;
            self.inspection["diagnostic_chunk_masks"] = json!(options.diagnostic_chunk_masks);
            fs::write(
                output.join("native-materials.json"),
                serde_json::to_vec_pretty(&assembly.material_provenance()?)?,
            )?;
            let mut head_min = glam::Vec3::splat(f32::INFINITY);
            let mut head_max = glam::Vec3::splat(f32::NEG_INFINITY);
            for node in self.inspection["nodes"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|n| {
                    n["name"]
                        .as_str()
                        .is_some_and(|name| head_names.contains(name))
                })
            {
                for chunk in node["chunks"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|c| c["visible"] == true)
                {
                    let min: [f32; 3] =
                        serde_json::from_value(chunk["world_bounds"]["min"].clone())?;
                    let max: [f32; 3] =
                        serde_json::from_value(chunk["world_bounds"]["max"].clone())?;
                    head_min = head_min.min(min.into());
                    head_max = head_max.max(max.into());
                }
            }
            self.head_bounds = (head_min.is_finite() && head_max.is_finite()).then(|| {
                (
                    (head_min + head_max) * 0.5,
                    (head_max - head_min).max_element() * 0.7,
                )
            });
            self.inspection["visual_tags"] = json!(plan.tags);
            self.inspection["source_diagnostics"] = json!(plan.source_diagnostics);
            let mut minimum = glam::Vec3::splat(f32::INFINITY);
            let mut maximum = glam::Vec3::splat(f32::NEG_INFINITY);
            for node in self.inspection["nodes"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|n| n["visible"] == true)
            {
                for chunk in node["chunks"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|c| c["visible"] == true)
                {
                    if let (Ok(min), Ok(max)) = (
                        serde_json::from_value::<[f32; 3]>(chunk["world_bounds"]["min"].clone()),
                        serde_json::from_value::<[f32; 3]>(chunk["world_bounds"]["max"].clone()),
                    ) {
                        minimum = minimum.min(min.into());
                        maximum = maximum.max(max.into());
                    }
                }
            }
            ensure!(
                minimum.is_finite() && maximum.is_finite(),
                "assembled character has no visible finite geometry"
            );
            self.bounds = (
                (minimum + maximum) * 0.5,
                ((maximum - minimum).x.max((maximum - minimum).y) * 0.54).max(0.05),
            );
            let rig = *assembly
                .rigs
                .get("root")
                .context("character locomotion rig absent")?;
            self.pose_rig = Some((rig, assembly.scene.skeleton(rig)?.clone()));
            self.prepared = Some(self.renderer.prepare_scene(&assembly.scene)?);
            self._preparation = Some(preparation);
            self.assembly_key = key;
        }
        let prepared = self
            .prepared
            .as_mut()
            .context("preview scene was not prepared")?;
        let requested = shape_weights(project, catalogue)?;
        let (rig, skeleton) = self.pose_rig.as_ref().context("preview skeleton absent")?;
        prepared.set_pose(*rig, &options.pose.sample(skeleton)?)?;
        self.inspection["pose"] =
            json!({"name":options.pose.name(),"kind":"synthetic skeletal stress pose"});
        if options.pose != crate::poses::CheckPose::Rest {
            fs::create_dir_all(output)?;
            fs::write(
                output.join("pose-collisions.json"),
                serde_json::to_vec_pretty(&crate::poses::inspect(&prepared.evaluated_geometry()?))?,
            )?;
        }
        let mut applied = std::collections::BTreeSet::new();
        for node in self.nodes.values() {
            let targets = match prepared.morph_targets(*node) {
                Ok(targets) => targets,
                Err(_) => continue,
            };
            let weights: Vec<f32> = targets
                .iter()
                .map(|target| {
                    if requested.contains_key(&target.name) {
                        applied.insert(target.name.clone());
                        1.
                    } else {
                        0.
                    }
                })
                .collect();
            if !weights.is_empty() {
                prepared.set_morph_weights(*node, &weights)?;
            }
        }
        for name in requested.keys() {
            ensure!(
                applied.contains(name),
                "selected assembly has no native morph target {name}; select a morph-bearing head before previewing this shape"
            );
        }
        ensure!(!cancelled(), "preview cancelled");
        fs::create_dir_all(output)?;
        if options.browser_scene {
            fs::write(output.join("scene.glb"), prepared.browser_glb()?)?;
            return Ok(
                json!({"revision":project.revision,"appearance":project.active_appearance,
                "scene":"scene.glb","head_bounds":self.head_bounds.map(|(center,radius)|json!({"center":center.to_array(),"radius":radius})),
                "warnings":validation.warnings,"adapter":"Browser GPU · native assembly"}),
            );
        }
        let camera = {
            let bounds = if options.focus_head {
                self.head_bounds.context("native head bounds absent")?
            } else {
                self.bounds
            };
            let target = bounds.0;
            let distance = 3.0 / options.zoom;
            let direction = glam::Vec3::new(
                options.yaw.sin() * options.pitch.cos(),
                options.pitch.sin(),
                -options.yaw.cos() * options.pitch.cos(),
            );
            Camera::Orthographic {
                eye: (target + direction * distance).to_array(),
                target: target.to_array(),
                up: [0., 1., 0.],
                half_height: bounds.1 / options.zoom,
                near: 0.001,
                far: 20.,
            }
        };
        let rendered = self.renderer.render(
            prepared,
            RenderOptions {
                size: options.size,
                camera,
            },
        )?;
        if let Ok(pixel) = std::env::var("GHOSTLINE_PROBE_PIXEL") {
            let pixel = pixel
                .split(',')
                .map(str::parse::<f32>)
                .collect::<std::result::Result<Vec<_>, _>>()?;
            ensure!(pixel.len() == 2, "probe pixel must be x,y");
            if let Camera::Orthographic {
                eye,
                target,
                up,
                half_height,
                ..
            } = camera
            {
                let direction =
                    (glam::Vec3::from_array(target) - glam::Vec3::from_array(eye)).normalize();
                let right = direction.cross(glam::Vec3::from_array(up)).normalize();
                let vertical = right.cross(direction);
                let origin = glam::Vec3::from_array(eye)
                    + right * ((pixel[0] + 0.5) / options.size as f32 - 0.5) * half_height * 2.
                    + vertical * (0.5 - (pixel[1] + 0.5) / options.size as f32) * half_height * 2.;
                self.inspection["surface_probe"] =
                    surface_probe(&prepared.evaluated_geometry()?, origin, direction);
            }
        }
        let image_path = output.join("preview.png");
        use image::ImageEncoder as _;
        let file = std::fs::File::create(&image_path)?;
        image::codecs::png::PngEncoder::new_with_quality(
            file,
            image::codecs::png::CompressionType::Fast,
            image::codecs::png::FilterType::Adaptive,
        )
        .write_image(
            rendered.image.as_raw(),
            options.size,
            options.size,
            image::ExtendedColorType::Rgba8,
        )?;
        fs::write(
            output.join("assembly.json"),
            serde_json::to_vec_pretty(&self.inspection)?,
        )?;
        Ok(
            json!({"revision":project.revision,"appearance":project.active_appearance,"image":"preview.png",
            "adapter":rendered.adapter.name,"selected":selections.iter().map(|s| &s["id"]).collect::<Vec<_>>(),
            "morphs":applied,"warnings":validation.warnings,"assembly":"assembly.json"}),
        )
    }
}

fn surface_probe(
    chunks: &[ghostline_renderer::EvaluatedChunk],
    origin: glam::Vec3,
    direction: glam::Vec3,
) -> Value {
    use glam::Vec3;
    let mut hits = Vec::new();
    for chunk in chunks {
        for (triangle, face) in chunk.indices.chunks_exact(3).enumerate() {
            let ids: [usize; 3] = std::array::from_fn(|i| usize::from(face[i]));
            let [a, b, c] = ids.map(|i| Vec3::from_array(chunk.positions[i]));
            let ab = b - a;
            let ac = c - a;
            let p = direction.cross(ac);
            let determinant = ab.dot(p);
            if determinant.abs() < 1e-9 {
                continue;
            }
            let delta = origin - a;
            let u = delta.dot(p) / determinant;
            let q = delta.cross(ab);
            let v = direction.dot(q) / determinant;
            let distance = ac.dot(q) / determinant;
            if u < 0. || v < 0. || u + v > 1. || distance < 0. {
                continue;
            }
            hits.push((distance,json!({"component":chunk.name,"mesh":chunk.mesh,"chunk":chunk.chunk,"triangle":triangle,"indices":ids,
                "distance":distance,"weights":[1.-u-v,u,v],"position":(origin+direction*distance).to_array(),"world_triangle":[a.to_array(),b.to_array(),c.to_array()],
                "normals":ids.map(|i|chunk.normals[i]),"bind_triangle":ids.map(|i|chunk.transforms[i].inverse().transform_point3(chunk.positions[i].into()).to_array()),
                "transforms":ids.map(|i|chunk.transforms[i].to_cols_array())})));
        }
    }
    hits.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut crossings = Vec::new();
    if let Some((distance, hit)) = hits.first() {
        let center = origin + direction * (*distance);
        let component = hit["component"].as_str().unwrap_or("");
        let chunk_id = hit["chunk"].as_u64().unwrap_or(0) as usize;
        let mut receiving = Vec::new();
        let mut owners = BTreeMap::new();
        for chunk in chunks.iter().filter(|chunk| chunk.name != component) {
            for (face_id, face) in chunk.indices.chunks_exact(3).enumerate() {
                let triangle = ghostline_refitter::Triangle(std::array::from_fn(|i| {
                    Vec3::from_array(chunk.positions[usize::from(face[i])])
                }));
                if triangle.0.iter().any(|point| point.distance(center) < 0.08) {
                    owners.insert(
                        triangle.0.map(|p| p.to_array().map(f32::to_bits)),
                        (chunk.name.clone(), chunk.chunk, face_id),
                    );
                    receiving.push(triangle);
                }
            }
        }
        let surface = ghostline_refitter::surface::Surface::new(&receiving);
        for chunk in chunks
            .iter()
            .filter(|chunk| chunk.name == component && chunk.chunk == chunk_id)
        {
            for (face_id, face) in chunk.indices.chunks_exact(3).enumerate() {
                let triangle = ghostline_refitter::Triangle(std::array::from_fn(|i| {
                    Vec3::from_array(chunk.positions[usize::from(face[i])])
                }));
                if !triangle.0.iter().any(|point| point.distance(center) < 0.08) {
                    continue;
                }
                for crossing in surface.crossings_triangle(triangle) {
                    let Some((other, other_chunk, other_face)) =
                        owners.get(&crossing.triangle.0.map(|p| p.to_array().map(f32::to_bits)))
                    else {
                        continue;
                    };
                    crossings.push(json!({"triangle":face_id,"indices":face,"other":other,"other_chunk":other_chunk,"other_triangle":other_face,"segment":crossing.segment.map(|p|p.to_array())}));
                }
            }
        }
    }
    json!({"origin":origin.to_array(),"direction":direction.to_array(),"proper_crossings":crossings,"hits":hits.into_iter().take(10).map(|(_,hit)|hit).collect::<Vec<_>>()})
}

pub fn review_wardrobe(
    catalogue: &Catalogue,
    game: &Path,
    schema: Option<&Path>,
    out: &Path,
) -> Result<()> {
    fs::create_dir_all(out)?;
    let mut session = Session::open(game, schema)?;
    let red_schema = ghostline_red::schema::RedSchema::load(Some(game), schema)?;
    let mut outcomes = Vec::new();
    for &template in templates::TEMPLATES
        .iter()
        .filter(|t| t.head_mode == "transfer")
    {
        let outfits = crate::frames::outfits(template)?;
        let outfit = outfits
            .iter()
            .find(|o| o["native_outfit"] == crate::frames::default_head(template.id))
            .or_else(|| outfits.first())
            .context("NPC outfit missing")?;
        let mut project = Project::new(&format!("wardrobe_{}", template.id), template.id)?;
        catalogue.defaults(&mut project)?;
        project.appearances[0].native_outfit = Some(
            outfit["native_outfit"]
                .as_str()
                .context("outfit identity missing")?
                .into(),
        );
        project.appearances[0].skin_tone = Some("05_bl_espresso".into());
        project::write(
            &out.join(format!("{}.character.json", project.id)),
            &project,
        )?;
        let directory = out.join(&project.id);
        let result = session
            .preview(
                &project,
                catalogue,
                &directory,
                &PreviewOptions::default(),
                || false,
            )
            .and_then(|preview| {
                let export = export_with_source(
                    &project,
                    catalogue,
                    &session.source,
                    &red_schema,
                    &directory.join("export"),
                    || false,
                )?;
                Ok(json!({"preview":preview,"export":export}))
            });
        let receipt = match result {
            Ok(result) => {
                json!({"template":template.id,"outfit":outfit,"ready":true,"result":result})
            }
            Err(error) => {
                json!({"template":template.id,"outfit":outfit,"ready":false,"error":format!("{error:#}")})
            }
        };
        println!(
            "{}: ready={} {}",
            template.id, receipt["ready"], receipt["error"]
        );
        outcomes.push(receipt);
        fs::write(
            out.join("review.json"),
            serde_json::to_vec_pretty(&outcomes)?,
        )?;
    }
    ensure!(
        outcomes.iter().all(|o| o["ready"] == true),
        "NPC wardrobe failures are listed in {}",
        out.join("review.json").display()
    );
    Ok(())
}

pub fn review(
    catalogue: &Catalogue,
    game: &Path,
    schema: Option<&Path>,
    out: &Path,
    exporting: bool,
) -> Result<()> {
    fs::create_dir_all(out)?;
    let mut session = Session::open(game, schema)?;
    let red_schema = ghostline_red::schema::RedSchema::load(Some(game), schema)?;
    let mut outcomes = Vec::new();
    if let Ok(bytes) = fs::read(out.join("review.json")) {
        outcomes = serde_json::from_slice::<Vec<Value>>(&bytes)?;
    }
    for &template in templates::TEMPLATES {
        let options = crate::frames::options(template)?;
        let alternate = options
            .as_array()
            .and_then(|options| options.iter().find(|h| h["default"] != true))
            .and_then(|h| h["id"].as_str());
        for variant in 0..3 {
            let unsupported_clothing = variant == 2
                && matches!(
                    template.id,
                    "female_big" | "male_big" | "female_fat" | "male_fat"
                );
            if outcomes.iter().any(|o| {
                o["template"] == template.id
                    && o["variant"] == variant
                    && (if unsupported_clothing {
                        o["expected_rejection"] == true
                    } else {
                        o["ready"] == true && (!exporting || o["export"]["ok"] == true)
                    })
            }) {
                continue;
            }
            let mut project =
                Project::new(&format!("review_{}_{variant}", template.id), template.id)?;
            catalogue.defaults(&mut project)?;
            if variant == 1 {
                if template.head_mode == "transfer" {
                    project.appearances[0].skin_tone = Some("05_bl_espresso".into());
                }
                project.appearances[0].shapes = std::collections::BTreeMap::from([
                    ("eyes".into(), 2),
                    ("nose".into(), 3),
                    ("mouth".into(), 4),
                    ("jaw".into(), 2),
                    ("ears".into(), 2),
                ]);
                project.appearances[0].native_head = alternate.map(str::to_owned);
            }
            if variant == 2 {
                for slot in ["Clo_InnerChest", "Clo_Legs", "Clo_Feet"] {
                    let entry = catalogue
                        .entries
                        .iter()
                        .find(|e| {
                            e["category"] == slot
                                && e["resolution"] == "resolved"
                                && e["frame"]
                                    .as_str()
                                    .is_none_or(|frame| frame == template.frame)
                                && e["appearance"]
                                    .as_str()
                                    .is_some_and(|a| !a.contains("FPP") && !a.contains("Part"))
                        })
                        .context("representative clothing absent")?;
                    project.appearances[0].equipment.insert(
                        slot.into(),
                        entry["id"]
                            .as_str()
                            .context("clothing identity absent")?
                            .into(),
                    );
                }
            }
            let directory = out.join(&project.id);
            project::write(
                &out.join(format!("{}.character.json", project.id)),
                &project,
            )?;
            if unsupported_clothing {
                let validation = catalogue.validate(&project);
                ensure!(!validation.ok, "incompatible clothing was accepted");
                outcomes.retain(|o| !(o["template"] == template.id && o["variant"] == variant));
                outcomes.push(json!({"template":template.id,"variant":variant,"ready":false,"expected_rejection":true,"validation":validation}));
                fs::write(
                    out.join("review.json"),
                    serde_json::to_vec_pretty(&outcomes)?,
                )?;
                println!(
                    "{} / {variant}: incompatible player clothing rejected",
                    template.id
                );
                continue;
            }
            let result = session.preview(
                &project,
                catalogue,
                &directory,
                &PreviewOptions::default(),
                || false,
            );
            let result = result.and_then(|mut result| {
                let options = PreviewOptions {
                    focus_head: true,
                    ..PreviewOptions::default()
                };
                let head = session.preview(
                    &project,
                    catalogue,
                    &directory.join("head"),
                    &options,
                    || false,
                )?;
                result["head"] = head;
                Ok(result)
            });
            let mut receipt = match result {
                Ok(result) => {
                    json!({"template":template.id,"variant":variant,"ready":true,"result":result})
                }
                Err(error) => {
                    json!({"template":template.id,"variant":variant,"ready":false,"error":format!("{error:#}")})
                }
            };
            if exporting && receipt["ready"] == true {
                receipt["export"] = match export_with_source(
                    &project,
                    catalogue,
                    &session.source,
                    &red_schema,
                    &directory.join("export"),
                    || false,
                ) {
                    Ok(report) => json!({"ok":true,"report":report}),
                    Err(error) => json!({"ok":false,"error":format!("{error:#}")}),
                };
            }
            println!(
                "{} / {}: preview={}, export={}",
                template.id, variant, receipt["ready"], receipt["export"]["ok"]
            );
            outcomes.retain(|o| !(o["template"] == template.id && o["variant"] == variant));
            outcomes.push(receipt);
            fs::write(
                out.join("review.json"),
                serde_json::to_vec_pretty(&outcomes)?,
            )?;
        }
    }
    ensure!(
        outcomes.iter().all(|o| o["expected_rejection"] == true
            || (o["ready"] == true && (!exporting || o["export"]["ok"] == true))),
        "body/head review failures are listed in {}",
        out.join("review.json").display()
    );
    Ok(())
}

pub fn build(
    project: &Project,
    catalogue: &Catalogue,
    game: &Path,
    schema: Option<&Path>,
    out: &Path,
    export: bool,
    cancelled: impl Fn() -> bool,
) -> Result<Value> {
    if export {
        return export_character(project, catalogue, game, schema, out, cancelled);
    }
    Session::open(game, schema)?.preview(
        project,
        catalogue,
        out,
        &PreviewOptions::default(),
        cancelled,
    )
}

fn export_character(
    project: &Project,
    catalogue: &Catalogue,
    game: &Path,
    schema: Option<&Path>,
    out: &Path,
    cancelled: impl Fn() -> bool,
) -> Result<Value> {
    let source = GameSource::open(game, schema)?;
    let schema = ghostline_red::schema::RedSchema::load(Some(game), schema)?;
    export_with_source(project, catalogue, &source, &schema, out, cancelled)
}

fn export_with_source(
    project: &Project,
    catalogue: &Catalogue,
    source: &GameSource,
    schema: &ghostline_red::schema::RedSchema,
    out: &Path,
    cancelled: impl Fn() -> bool,
) -> Result<Value> {
    let validation = catalogue.validate(project);
    ensure!(validation.ok, "{}", validation.errors.join("; "));
    ensure!(
        !out.exists(),
        "export output already exists: {}",
        out.display()
    );
    ensure!(!cancelled(), "export cancelled");
    let template = templates::get(&project.template)?;
    let mut entity = templates::shell(template, "ent")?;
    let mut app = templates::shell(template, "app")?;
    let mut rows = Vec::new();
    let mut mappings = Vec::new();
    let mut handle = 0_u64;
    let mut warnings = validation.warnings;
    let mut source_diagnostics = Vec::new();
    let mut dependencies = std::collections::BTreeMap::new();
    let mut dependency_origins = std::collections::BTreeMap::new();
    let mut local_resources = std::collections::BTreeSet::new();
    let staging = tempfile::tempdir()?;
    let archive_root = staging.path().join("source/archive");
    let depot = archive_root.join(project.namespace.replace('\\', "/"));
    fs::create_dir_all(depot.join("head"))?;
    for appearance in &project.appearances {
        let mut selected_project = project.clone();
        selected_project
            .active_appearance
            .clone_from(&appearance.id);
        let weights = shape_weights(&selected_project, catalogue)?;
        let mut components = Vec::new();
        let mut cruids = serde_json::Map::new();
        let mut baked_targets = std::collections::BTreeSet::new();
        let mut names = std::collections::BTreeSet::new();
        {
            let plan = crate::assembly::resolve(&selected_project, catalogue, source)?;
            source_diagnostics.extend(plan.source_diagnostics.iter().cloned());
            for mut component in plan
                .native
                .components
                .into_iter()
                .filter(|c| !plan.root_names.contains(&c.name))
            {
                let mut data = crate::assembly::component_document(&component);
                let Some(mesh) = component.mesh else {
                    renumber_handles(&mut data, &mut handle)?;
                    let cruid = data["id"]
                        .as_u64()
                        .or_else(|| data["id"].as_str().and_then(|id| id.parse().ok()))
                        .filter(|id| *id != 0)
                        .unwrap_or_else(|| {
                            ResourceKey::parse(&format!(
                                "{}\\{}\\{}",
                                project.namespace, appearance.id, component.name
                            ))
                            .native_hash()
                        });
                    data["id"] = json!(cruid.to_string());
                    cruids.insert(components.len().to_string(), json!(cruid.to_string()));
                    components.push(data);
                    continue;
                };
                let document = if let Some(document) = plan.modified_meshes.get(&mesh) {
                    std::sync::Arc::new(document.clone())
                } else {
                    source.resources().document(&mesh)?
                };
                let (mesh_value, kind) = if component.enabled
                    && component.chunk_mask != 0
                    && (document["Data"]["RootChunk"]["$type"] == "MorphTargetMesh"
                        || plan.transfers.contains_key(&mesh)
                        || plan.modified_meshes.contains_key(&mesh))
                {
                    let native_morph = document["Data"]["RootChunk"]["$type"] == "MorphTargetMesh";
                    let base = if native_morph {
                        ghostline_red::morph::base_mesh(&document)?
                    } else {
                        mesh.clone()
                    };
                    let base_document = if base == mesh {
                        std::sync::Arc::clone(&document)
                    } else {
                        source.resources().document(&base)?
                    };
                    let baked_appearance = source
                        .resources()
                        .mesh_appearance_binding(&base, &component.mesh_appearance)?
                        .appearance;
                    component.mesh_appearance.clone_from(&baked_appearance);
                    let targets = if native_morph {
                        ghostline_red::morph::decode(&document, &base_document)?.targets
                    } else if plan.transfers.contains_key(&mesh) {
                        let chunks = ghostline_red::mesh::decode_geometry(&base_document, false)?;
                        transferred_targets(source, &plan.transfers[&mesh], &mesh, &chunks)?
                    } else {
                        Vec::new()
                    };
                    let selected_weights = weights
                        .iter()
                        .filter(|(name, _)| targets.iter().any(|t| t.name == **name))
                        .map(|(k, v)| (k.clone(), *v))
                        .collect();
                    let baked = if native_morph {
                        ghostline_red::morph::bake(&document, &base_document, &selected_weights)?
                    } else {
                        let mut baked = base_document.as_ref().clone();
                        let mut chunks = ghostline_red::mesh::decode_geometry(&baked, false)?;
                        for target in targets
                            .iter()
                            .filter(|t| selected_weights.contains_key(&t.name))
                        {
                            for change in &target.chunks {
                                let chunk = chunks
                                    .iter_mut()
                                    .find(|c| c.chunk_index == change.chunk)
                                    .context("transferred morph chunk missing")?;
                                for (&vertex, &delta) in
                                    change.vertices.iter().zip(&change.positions)
                                {
                                    for (axis, offset) in delta.into_iter().enumerate() {
                                        chunk.positions[vertex as usize][axis] +=
                                            offset * selected_weights[&target.name];
                                    }
                                }
                            }
                        }
                        if !selected_weights.is_empty() {
                            ghostline_red::mesh::replace_geometry(&mut baked, &chunks)?;
                        }
                        baked
                    };
                    baked_targets.extend(selected_weights.keys().cloned());
                    let file = format!("{}_{}.mesh", appearance.id, mesh.native_hash());
                    let raw = staging.path().join(format!("{file}.json"));
                    fs::write(&raw, serde_json::to_vec(&baked)?)?;
                    let binary = depot.join("head").join(&file);
                    let template_path = staging.path().join(format!("{file}.template"));
                    fs::write(&template_path, source.resources().read_bytes(&base)?)?;
                    ghostline_red::writer::write_with_red_schema(
                        &raw,
                        &template_path,
                        &binary,
                        schema,
                    )?;
                    let readback =
                        ghostline_red::codec::decode_wkit_from_bytes(&fs::read(&binary)?, schema)?;
                    let decoded = ghostline_red::mesh::decode_geometry(&readback, false)?;
                    collect_resources(&readback, &mut dependencies, &mut dependency_origins)?;
                    let expected_materials =
                        ghostline_red::material::local_material_documents(&baked, schema)?;
                    let actual_materials =
                        ghostline_red::material::local_material_documents(&readback, schema)?;
                    ensure!(
                        expected_materials.len() == actual_materials.len(),
                        "written mesh local material count differs"
                    );
                    for (expected, actual) in expected_materials.iter().zip(&actual_materials) {
                        ensure!(
                            expected["Data"]["RootChunk"]["values"]
                                == actual["Data"]["RootChunk"]["values"],
                            "written mesh material parameters differ"
                        );
                    }
                    for actual in ghostline_red::material::active_local_material_documents(
                        &readback,
                        &baked_appearance,
                        schema,
                    ).with_context(|| format!("export material readback for {} ({mesh}), appearance {baked_appearance}", component.name))? {
                        let mut scoped = actual.clone();
                        scoped["Data"]["EmbeddedFiles"] = readback["Data"]["EmbeddedFiles"].clone();
                        collect_resources(&scoped, &mut dependencies, &mut dependency_origins)?;
                    }
                    local_resources.insert(
                        ResourceKey::parse(&format!("{}\\head\\{file}", project.namespace))
                            .native_hash(),
                    );
                    let expected = ghostline_red::mesh::decode_geometry(&baked, false)?;
                    ensure!(
                        decoded.len() == expected.len()
                            && decoded.iter().zip(&expected).all(|(a, b)| a.positions.len()
                                == b.positions.len()
                                && a.positions.iter().zip(&b.positions).all(|(a, b)| a
                                    .iter()
                                    .zip(b)
                                    .all(|(a, b)| (a - b).abs() <= 0.00001))
                                && a.indices == b.indices
                                && a.skin.len() == b.skin.len()
                                && a.skin
                                    .iter()
                                    .zip(&b.skin)
                                    .all(|(a, b)| a.joints == b.joints && a.weights == b.weights)),
                        "baked mesh {} changed during native writing: expected chunks {}, decoded {}, first positions {:?} / {:?}",
                        mesh,
                        expected.len(),
                        decoded.len(),
                        expected.first().and_then(|c| c.positions.first()),
                        decoded.first().and_then(|c| c.positions.first())
                    );
                    (
                        typed(
                            "ResourcePath",
                            &format!("{}\\head\\{file}", project.namespace),
                        ),
                        "entSkinnedMeshComponent".to_owned(),
                    )
                } else {
                    let value = match &mesh {
                        ResourceKey::Path(path) => typed("ResourcePath", path),
                        ResourceKey::Hash(hash) => {
                            json!({"$type":"ResourcePath","$storage":"uint64","$value":hash.to_string()})
                        }
                    };
                    (value, component.kind)
                };
                let name = component.name;
                ensure!(
                    names.insert(name.clone()),
                    "selected appearances provide conflicting component {name}; resolve their activation before exporting"
                );
                let (scale, rotation, translation) =
                    component.local.to_scale_rotation_translation();
                ensure!(
                    data.is_object(),
                    "native component template is absent: {name}"
                );
                data["$type"] = json!(kind);
                data["name"] = typed("CName", &name);
                data.as_object_mut()
                    .context("component template is invalid")?
                    .remove("morphResource");
                data["mesh"] = json!({"DepotPath":mesh_value,"Flags":"Default"});
                data["meshAppearance"] = typed("CName", &component.mesh_appearance);
                data["chunkMask"] = json!(component.chunk_mask.to_string());
                data["isEnabled"] = json!(u8::from(component.enabled));
                let mut position = json!({"$type":"WorldPosition"});
                for (axis, value) in ["x", "y", "z"].into_iter().zip(translation.to_array()) {
                    let bits = (f64::from(value) * 131072.).round();
                    ensure!(
                        (i32::MIN as f64..=i32::MAX as f64).contains(&bits),
                        "component position exceeds native fixed-point range"
                    );
                    position[axis] = json!({"$type":"FixedPoint","Bits":bits as i32});
                }
                data["localTransform"] = json!({"$type":"WorldTransform","Position":position,"Orientation":{"$type":"Quaternion","i":rotation.x,"j":rotation.y,"k":rotation.z,"r":rotation.w}});
                data["visualScale"] =
                    json!({"$type":"Vector3","X":scale.x,"Y":scale.y,"Z":scale.z});
                for (field, binding) in [
                    ("parentTransform", &component.parent),
                    ("controlBinding", &component.control),
                ] {
                    if !data[field].is_null() {
                        data[field]["Data"]["bindName"] = typed("CName", &binding.component);
                    }
                }
                renumber_handles(&mut data, &mut handle)?;
                let cruid = data["id"]
                    .as_u64()
                    .or_else(|| data["id"].as_str().and_then(|id| id.parse().ok()))
                    .filter(|id| *id != 0)
                    .unwrap_or_else(|| {
                        ResourceKey::parse(&format!(
                            "{}\\{}\\{name}",
                            project.namespace, appearance.id
                        ))
                        .native_hash()
                    });
                data["id"] = json!(cruid.to_string());
                cruids.insert(components.len().to_string(), json!(cruid.to_string()));
                components.push(data);
            }
        }
        // Match the original builder's appearance controller and native component table.
        let dependencies = components
            .iter()
            .filter(|component| component.get("mesh").is_some())
            .map(|component| {
                json!({
                    "$type":"entVisualControllerDependency",
                    "componentName":component["name"],
                    "appearanceName":component["meshAppearance"],
                    "mesh":component["mesh"]
                })
            })
            .collect::<Vec<_>>();
        if let Some(controller) = components
            .iter_mut()
            .find(|component| component["name"]["$value"] == "AppearanceVisualController")
        {
            controller["appearanceDependency"] = json!(dependencies);
        } else {
            let controller_id = "18446744073709550000";
            cruids.insert(components.len().to_string(), json!(controller_id));
            components.push(json!({"$type":"entVisualControllerComponent",
                "name":typed("CName","AppearanceVisualController"),
                "id":controller_id,"appearanceDependency":dependencies}));
        }
        for name in weights.keys() {
            ensure!(
                baked_targets.contains(name),
                "selected head has no exportable native morph target {name}"
            );
        }
        ensure!(
            !components.is_empty(),
            "appearance {} has no enabled mesh components",
            appearance.id
        );
        handle = handle
            .checked_add(1)
            .context("appearance handle overflow")?;
        rows.push(json!({"HandleId":handle.to_string(),"Data":{"$type":"appearanceAppearanceDefinition","name":typed("CName",&appearance.id),"components":components,
            "compiledData":{"BufferId":(rows.len()+1).to_string(),"Flags":0,"Type":"WolvenKit.RED4.Types.RedPackage","Data":{"Version":4,"Sections":7,"CruidIndex":-1,"CruidDict":cruids,"Chunks":components}}}}));
        handle += 1;
        mappings.push(json!({"$type":"entTemplateAppearance","name":typed("CName",&appearance.id),"appearanceName":typed("CName",&appearance.id),
            "appearanceResource":{"DepotPath":typed("ResourcePath",&format!("{}\\{}.app", project.namespace,project.id)),"Flags":"Soft"}}));
    }
    app["Data"]["RootChunk"]["appearances"] = Value::Array(rows);
    // These async references must not eagerly load the entity and appearance into each other.
    app["Data"]["RootChunk"]["baseEntity"] = json!({"DepotPath":typed("ResourcePath",&format!("{}\\{}.ent",project.namespace,project.id)),"Flags":"Soft"});
    entity["Data"]["RootChunk"]["appearances"] = Value::Array(mappings);
    entity["Data"]["RootChunk"]["defaultAppearance"] = typed("CName", &project.active_appearance);
    let mut root_dependencies = std::collections::BTreeMap::new();
    collect_resources(&entity, &mut root_dependencies, &mut dependency_origins)?;
    entity["Data"]["RootChunk"]["resolvedDependencies"]=Value::Array(root_dependencies.values().map(|resource|{
        let path=match resource {ResourceKey::Path(path)=>typed("ResourcePath",path),ResourceKey::Hash(hash)=>json!({"$type":"ResourcePath","$storage":"uint64","$value":hash.to_string()})};
        json!({"DepotPath":path,"Flags":"Soft"})
    }).collect());
    for (kind, document) in [("ent", entity), ("app", app)] {
        ensure!(!cancelled(), "export cancelled");
        let raw = staging.path().join(format!("{}.{}.json", project.id, kind));
        fs::write(&raw, serde_json::to_vec_pretty(&document)?)?;
        let binary = depot.join(format!("{}.{}", project.id, kind));
        ghostline_red::writer::write(&raw, &binary, schema)?;
        let decoded = ghostline_red::codec::decode_wkit_from_bytes(&fs::read(&binary)?, schema)?;
        collect_resources(&decoded, &mut dependencies, &mut dependency_origins)?;
        local_resources.insert(
            ResourceKey::parse(&format!("{}\\{}.{}", project.namespace, project.id, kind))
                .native_hash(),
        );
        ensure!(
            decoded["Data"]["RootChunk"]["$type"] == document["Data"]["RootChunk"]["$type"],
            "resource readback root differs"
        );
        if kind == "app" {
            ensure!(
                decoded["Data"]["RootChunk"]["baseEntity"]["Flags"] == "Soft",
                "written appearance base entity must remain an async reference"
            );
        }
        let expected = document["Data"]["RootChunk"]["appearances"]
            .as_array()
            .context("authored appearance list absent")?;
        let actual = decoded["Data"]["RootChunk"]["appearances"]
            .as_array()
            .context("written appearance list absent")?;
        ensure!(
            actual.len() == expected.len(),
            "written appearance count differs"
        );
        for (a, b) in actual.iter().zip(expected) {
            let a = a.get("Data").unwrap_or(a);
            let b = b.get("Data").unwrap_or(b);
            ensure!(
                a["name"]["$value"] == b["name"]["$value"],
                "written appearance identity differs"
            );
            if kind == "ent" {
                ensure!(
                    a["appearanceResource"]["Flags"] == "Soft",
                    "written entity appearance must remain an async reference"
                );
            }
            if kind == "app" {
                let cruids = &a["compiledData"]["Data"]["CruidDict"];
                let chunks = a["compiledData"]["Data"]["Chunks"]
                    .as_array()
                    .context("written component table absent")?;
                for (index, component) in chunks.iter().enumerate() {
                    ensure!(
                        component["id"] == cruids[index.to_string()],
                        "written component ID disagrees with compiled table: {}",
                        component["name"]
                    );
                }
                let a = a
                    .pointer("/compiledData/Data/Chunks")
                    .and_then(Value::as_array)
                    .context("written appearance package absent")?;
                let b = b
                    .pointer("/compiledData/Data/Chunks")
                    .and_then(Value::as_array)
                    .context("authored appearance package absent")?;
                ensure!(a.len() == b.len(), "written component count differs");
                for (a, b) in a.iter().zip(b) {
                    ensure!(
                        a["name"]["$value"] == b["name"]["$value"]
                            && (b.get("mesh").is_none() || a["chunkMask"] == b["chunkMask"]),
                        "written component identity or visibility differs: class={}, expected name={}, actual name={}, expected mask={}, actual mask={}",
                        b["$type"],
                        b["name"],
                        a["name"],
                        b["chunkMask"],
                        a["chunkMask"]
                    );
                }
            }
        }
    }
    let mut missing = Vec::new();
    for (hash, resource) in &dependencies {
        if !local_resources.contains(hash) && !source.resources().resource_exists(resource) {
            missing.push(format!(
                "{} at {}",
                resource,
                dependency_origins
                    .get(hash)
                    .and_then(|v| v.first())
                    .cloned()
                    .unwrap_or_default()
            ))
        }
    }
    ensure!(
        missing.is_empty(),
        "export has missing native dependencies: {}",
        missing.join(", ")
    );
    fs::create_dir_all(out.parent().unwrap_or(Path::new(".")))?;
    fs::create_dir(out)?;
    let archive = out.join(format!("{}.archive", project.id));
    ghostline_red::archive::pack(&archive_root, &archive)?;
    project::write(&out.join(format!("{}.character.json", project.id)), project)?;
    let tweaks = out.join(format!("{}.yaml", project.id));
    let tweak = crate::tweaks::render(project, catalogue)?;
    fs::write(&tweaks, tweak)?;
    warnings.push("In-game appearance, deformation and fit still require verification".into());
    let report = json!({"project":project.id,"revision":project.revision,"archive":archive,"tweaks":tweaks,"warnings":warnings,"source_diagnostics":source_diagnostics,"verified_resources":local_resources.len(),"native_dependencies":dependencies.len(),"appearances":project.appearances.iter().map(|a|&a.id).collect::<Vec<_>>(),"game_content":source.content_identity()});
    fs::write(
        out.join("export-report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    Ok(report)
}

fn shape_weights(project: &Project, catalogue: &Catalogue) -> Result<BTreeMap<String, f32>> {
    let template = templates::get(&project.template)?;
    let root = if template.frame == "female" {
        "base\\gameplay\\gui\\fullscreen\\main_menu\\female_cco.inkcharcustomization"
    } else {
        "base\\gameplay\\gui\\fullscreen\\main_menu\\male_cco.inkcharcustomization"
    };
    let mut weights = BTreeMap::new();
    for (region, index) in &project.appearance().shapes {
        let native_region = if region == "ears" {
            "ear"
        } else {
            region.as_str()
        };
        let control = catalogue
            .controls
            .values()
            .find(|c| c["source"]["Path"] == root && c["native"]["name"] == native_region)
            .with_context(|| format!("missing native {region} control"))?;
        let choice = control["native"]["data"]["morphNames"]
            .as_array()
            .and_then(|choices| choices.get((*index - 1) as usize))
            .with_context(|| format!("missing native {region} choice {index}"))?;
        if let Some(name) = choice["morphName"]["$value"]
            .as_str()
            .filter(|name| !name.is_empty() && *name != "None")
        {
            weights.insert(name.to_owned(), 1.);
        }
    }
    Ok(weights)
}

fn collect_resources(
    value: &Value,
    output: &mut std::collections::BTreeMap<u64, ResourceKey>,
    origins: &mut std::collections::BTreeMap<u64, Vec<String>>,
) -> Result<()> {
    let embedded = value
        .pointer("/Data/EmbeddedFiles")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|file| {
            ResourceKey::from_value(&file["FileName"])
                .ok()
                .flatten()
                .map(|key| key.native_hash())
        })
        .collect::<std::collections::BTreeSet<_>>();
    visit_resources(value, output, &embedded, "", origins)
}

fn visit_resources(
    value: &Value,
    output: &mut std::collections::BTreeMap<u64, ResourceKey>,
    embedded: &std::collections::BTreeSet<u64>,
    path: &str,
    origins: &mut std::collections::BTreeMap<u64, Vec<String>>,
) -> Result<()> {
    match value {
        Value::Object(fields) => {
            if fields.get("$type").and_then(Value::as_str) == Some("ResourcePath")
                && let Some(resource) = ResourceKey::from_value(value)?
            {
                if let ResourceKey::Path(path) = &resource {
                    ensure!(
                        !path.to_lowercase().starts_with("tutorial\\"),
                        "export retains a tutorial dependency: {path}"
                    );
                }
                if !embedded.contains(&resource.native_hash()) {
                    origins
                        .entry(resource.native_hash())
                        .or_default()
                        .push(path.into());
                    output.insert(resource.native_hash(), resource);
                }
            }
            let label = fields
                .get("name")
                .and_then(|v| v["$value"].as_str())
                .map(|name| format!("{path}[{name}]"))
                .unwrap_or_else(|| path.into());
            for (key, value) in fields {
                visit_resources(value, output, embedded, &format!("{label}/{key}"), origins)?
            }
        }
        Value::Array(values) => {
            for (index, value) in values.iter().enumerate() {
                visit_resources(value, output, embedded, &format!("{path}/{index}"), origins)?
            }
        }
        _ => {}
    }
    Ok(())
}

fn transferred_targets(
    source: &GameSource,
    morph_source: &ResourceKey,
    mesh: &ResourceKey,
    chunks: &[ghostline_red::mesh::MeshChunk],
) -> Result<Vec<ghostline_red::morph::MorphTarget>> {
    let geometry = source.load_geometry(morph_source)?;
    if geometry.morph_targets().is_empty() {
        return Ok(Vec::new());
    }
    let document = source.resources().document(mesh)?;
    let neck = if crate::assembly::mesh_role(mesh) == Some("head") {
        let skin = ghostline_red::mesh::decode_skin(&document)?
            .context("native head has no skin binding for collar protection")?;
        Some(
            skin.names
                .iter()
                .position(|n| n == "Neck1")
                .map(|i| skin.inverse_bind[i].inverse().w_axis.y)
                .context("native head has no Neck1 anchor")?,
        )
    } else {
        None
    };
    Ok(ghostline_red::morph_transfer::transfer(
        geometry.chunks(),
        geometry.morph_targets(),
        chunks,
        neck,
    )?)
}

fn typed(kind: &str, value: &str) -> Value {
    json!({"$type":kind,"$storage":"string","$value":value})
}

fn renumber_handles(value: &mut Value, next: &mut u64) -> Result<()> {
    fn collect(value: &Value, ids: &mut std::collections::BTreeSet<String>) {
        match value {
            Value::Object(fields) => {
                if let Some(id) = fields.get("HandleId").and_then(Value::as_str) {
                    ids.insert(id.into());
                }
                for value in fields.values() {
                    collect(value, ids);
                }
            }
            Value::Array(values) => {
                for value in values {
                    collect(value, ids);
                }
            }
            _ => {}
        }
    }
    fn replace(value: &mut Value, mapping: &BTreeMap<String, String>) -> Result<()> {
        match value {
            Value::Object(fields) => {
                for key in ["HandleId", "HandleRefId"] {
                    if let Some(id) = fields.get(key).and_then(Value::as_str) {
                        if let Some(new) = mapping.get(id) {
                            fields.insert(key.into(), json!(new));
                        } else {
                            ensure!(
                                id == "0" || id == "-1",
                                "native component refers to an unexpanded handle {id}"
                            );
                        }
                    }
                }
                for value in fields.values_mut() {
                    replace(value, mapping)?;
                }
            }
            Value::Array(values) => {
                for value in values {
                    replace(value, mapping)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    let mut ids = std::collections::BTreeSet::new();
    collect(value, &mut ids);
    let mut mapping = BTreeMap::new();
    for id in ids {
        *next = next.checked_add(1).context("handle identity overflow")?;
        mapping.insert(id, next.to_string());
    }
    replace(value, &mapping)
}
