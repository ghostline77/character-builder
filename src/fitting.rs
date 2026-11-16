//! Native garment layering shared by preview and export.
mod cache;
mod native_contacts;
mod overlays;
mod parts;
mod visibility;

use crate::{catalogue::Catalogue, project::Project};
use anyhow::Result;
use ghostline_red::{
    assets::{Assembly, ResourceKey},
    mesh::MeshChunk,
};
use ghostline_refitter::{
    Triangle,
    layers::{SupportPath, fit_mesh_between},
};
use ghostline_renderer::{AssemblyOptions, EvaluatedChunk, GameSource, SceneAssembly};
use glam::Vec3;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap},
    sync::{LazyLock, Mutex},
};

struct Layer {
    owner: String,
    name: String,
    mesh: ResourceKey,
    score: i32,
    mask: u64,
    document: Value,
    chunks: Vec<MeshChunk>,
    rest_space: bool,
    original: Vec<MeshChunk>,
    overlays: std::collections::BTreeSet<usize>,
    attached: std::collections::BTreeSet<usize>,
    occluders: std::collections::BTreeSet<usize>,
}
#[derive(Clone)]
struct CachedFit {
    positions: Vec<Vec<[f32; 3]>>,
    changed: usize,
    maximum: f32,
}
static FITS: LazyLock<Mutex<BTreeMap<String, CachedFit>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));

fn text(value: &Value) -> Option<&str> {
    value.get("$value").unwrap_or(value).as_str()
}

/// Native score table from the garment-support contract. Body components are
/// fixed references, not candidates for deformation across entity boundaries.
fn prefix_score(name: &str) -> Option<i32> {
    match name.split('_').next()? {
        "s1" => Some(50),
        "l1" => Some(60),
        "t1" => Some(70),
        "i1" => Some(80),
        "hh" => Some(90),
        "h1" => Some(100),
        "h2" => Some(110),
        "t2" => Some(120),
        _ => None,
    }
}

fn schema_offset(source: &GameSource, selection: &Value) -> Result<i32> {
    let resource: ResourceKey = serde_json::from_value(selection["resource"].clone())?;
    let root = source.resources().document(&resource)?;
    if matches!(
        root["Data"]["RootChunk"]["$type"].as_str(),
        Some("CMesh" | "MorphTargetMesh")
    ) {
        return Ok(0);
    }
    let definition =
        crate::assembly::definition(source, &resource, selection["appearance"].as_str())?;
    let mut roots = vec![source.resources().document(&resource)?];
    for part in definition["partsValues"].as_array().into_iter().flatten() {
        if let Some(key) = ResourceKey::from_value(&part.get("Data").unwrap_or(part)["resource"])? {
            roots.push(source.resources().document(&key)?);
        }
    }
    let mut offset = 0;
    for document in roots {
        for tag in document["Data"]["RootChunk"]["visualTagsSchema"]["Data"]["visualTags"]["tags"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(text)
        {
            offset = match tag {
                "PlayerBodyPart" => -2000,
                "Tight" => -1000,
                "Large" => 1000,
                "XLarge" => 2000,
                _ => offset,
            };
        }
    }
    Ok(offset)
}
fn triangles(layer: &Layer, evaluated: Option<&[EvaluatedChunk]>) -> Vec<Triangle> {
    layer
        .chunks
        .iter()
        .filter(|c| {
            c.chunk_index < 64
                && layer.mask & (1 << c.chunk_index) != 0
                && !layer.overlays.contains(&c.chunk_index)
                && layer.document["Data"]["RootChunk"]["renderResourceBlob"]["Data"]["header"]
                    ["renderChunkInfos"][c.chunk_index]["lodMask"]
                    .as_u64()
                    == Some(1)
        })
        .flat_map(|c| {
            let frames = evaluated
                .into_iter()
                .flatten()
                .find(|frame| frame.name == layer.name && frame.chunk == c.chunk_index);
            c.indices.chunks_exact(3).filter_map(move |indices| {
                if evaluated.is_some() && frames.is_none() {
                    return None;
                }
                Some(Triangle(std::array::from_fn(|i| {
                    let vertex = usize::from(indices[i]);
                    frames.map_or(c.positions[vertex].into(), |frame| {
                        frame.transforms[vertex].transform_point3(c.positions[vertex].into())
                    })
                })))
            })
        })
        .collect()
}

fn paths(layer: &Layer, native: &Assembly) -> Result<Vec<Vec<SupportPath>>> {
    let support = ghostline_red::garment::decode(&layer.document)?;
    let inline = ghostline_red::mesh::decode_garment_morphs(&layer.document)?;
    let skin = ghostline_red::mesh::decode_skin(&layer.document)?;
    let mut segments = Vec::new();
    if let Some(skin) = &skin {
        let positions: HashMap<_, _> = skin
            .names
            .iter()
            .zip(&skin.inverse_bind)
            .map(|(name, matrix)| (name.as_str(), matrix.inverse().w_axis.truncate()))
            .collect();
        for rig in native.rigs.values() {
            for bone in &rig.bones {
                if let Some(parent) = bone.parent.and_then(|i| rig.bones.get(i))
                    && let Some((&a, &b)) = positions
                        .get(parent.name.as_str())
                        .zip(positions.get(bone.name.as_str()))
                    && a.distance(b) > 0.03
                {
                    segments.push((a, b));
                }
            }
        }
    }
    let mut result = Vec::new();
    for chunk in &layer.chunks {
        let morph = inline.iter().find(|m| m.chunk_index == chunk.chunk_index);
        let proxy = support
            .iter()
            .find(|s| s.custom_morph && s.positions.len() == chunk.positions.len());
        let mut directions = Vec::with_capacity(chunk.positions.len());
        for (index, point) in chunk.positions.iter().enumerate() {
            let point = Vec3::from_array(*point);
            let authored = morph.map(|m| m.offsets[index]).or_else(|| {
                proxy.and_then(|s| {
                    (s.positions[index].distance(point) < 0.003).then_some(s.offsets[index])
                })
            });
            let delta = if let Some(delta) = authored {
                delta
            } else {
                let center = segments
                    .iter()
                    .map(|&(a, b)| {
                        a + (b - a)
                            * ((point - a).dot(b - a) / (b - a).length_squared()).clamp(0., 1.)
                    })
                    .min_by(|a, b| {
                        a.distance_squared(point)
                            .total_cmp(&b.distance_squared(point))
                    });
                center.map_or(Vec3::ZERO, |center| center - point)
            };
            let length = delta.length();
            directions.push(SupportPath {
                direction: delta,
                maximum: if authored.is_some() {
                    length
                } else {
                    length.min(0.1)
                },
            });
        }
        result.push(directions);
    }
    Ok(result)
}

pub fn fit_layers(
    project: &Project,
    catalogue: &Catalogue,
    native: &mut Assembly,
    groups: &BTreeMap<String, Vec<String>>,
    source: &GameSource,
    modified: &mut HashMap<ResourceKey, Value>,
    diagnostics: &mut Vec<Value>,
) -> Result<()> {
    let mut layers = Vec::new();
    for id in project.appearance().equipment.values() {
        let Some(selection) = catalogue.selections.get(id) else {
            continue;
        };
        let offset = schema_offset(source, selection)?;
        let Some(names) = groups.get(id) else {
            continue;
        };
        for component in native
            .components
            .iter()
            .filter(|c| names.contains(&c.name) && c.enabled && c.chunk_mask != 0)
        {
            let Some(mesh) = &component.mesh else {
                continue;
            };
            let original = text(&component.template["name"]).unwrap_or(&component.name);
            let prefix = prefix_score(original).or_else(|| match mesh {
                ResourceKey::Path(path) => prefix_score(path.rsplit('\\').next().unwrap_or("")),
                _ => None,
            });
            let Some(score) = prefix else { continue };
            let rest_space = component.parent.component == "root"
                && component.local.abs_diff_eq(glam::Mat4::IDENTITY, 1e-5);
            if source
                .resources()
                .mesh_appearance_binding(mesh, &component.mesh_appearance)?
                .shadow_only
            {
                continue;
            }
            let document = modified.get(mesh).cloned().map_or_else(
                || {
                    source
                        .resources()
                        .document(mesh)
                        .map(|d| d.as_ref().clone())
                },
                Ok,
            )?;
            if document["Data"]["RootChunk"]["$type"] != "CMesh" {
                continue;
            }
            let chunks = ghostline_red::mesh::decode_geometry(&document, false)?;
            let (overlays, occluders) =
                overlays::classify(source, &document, &component.mesh_appearance, &chunks)?;
            let attached = overlays::receivers(&chunks, &overlays);
            layers.push(Layer {
                owner: id.clone(),
                name: component.name.clone(),
                mesh: mesh.clone(),
                score: score + offset,
                mask: component.chunk_mask,
                document,
                original: chunks.clone(),
                chunks,
                rest_space,
                overlays,
                attached,
                occluders,
            });
        }
    }
    parts::resolve(project, &mut layers, modified, diagnostics)?;
    layers.sort_by_key(|layer| std::cmp::Reverse(layer.score));
    // Enclosed feet must stop constraining a closed shoe's sole before fitting,
    // rather than distorting the shoe and hiding those feet afterwards.
    hide_covered_feet(native, &layers, source, diagnostics)?;
    // Native meshes can share a root component without sharing a bind space.
    // Use the same calibrated skin transforms as preview/export evaluation.
    let assembly = SceneAssembly::from_native(
        native.clone(),
        &AssemblyOptions::default(),
        |resource, _| {
            let geometry = source.load_geometry(resource)?;
            let document = modified.get(resource).or_else(|| {
                layers
                    .iter()
                    .find(|layer| &layer.mesh == resource)
                    .map(|layer| &layer.document)
            });
            let geometry = if let Some(document) = document {
                geometry.with_chunks(ghostline_red::mesh::decode_geometry(document, false)?)
            } else {
                geometry
            };
            Ok(std::sync::Arc::new(geometry.with_neutral_material()?))
        },
    )?;
    let evaluated = assembly.scene.evaluated_geometry()?;
    let mut body = Vec::new();
    for component in native
        .components
        .iter()
        .filter(|c| c.enabled && c.chunk_mask != 0)
    {
        if !visibility::body(&component.name) {
            continue;
        }
        let Some(mesh) = &component.mesh else {
            continue;
        };
        if source
            .resources()
            .mesh_appearance_binding(mesh, &component.mesh_appearance)?
            .shadow_only
        {
            continue;
        }
        let document = source.resources().document(mesh)?;
        for chunk in evaluated.iter().filter(|chunk| chunk.name == component.name &&
            document["Data"]["RootChunk"]["renderResourceBlob"]["Data"]["header"]["renderChunkInfos"][chunk.chunk]["lodMask"].as_u64() == Some(1))
        {
            for triangle in chunk.indices.chunks_exact(3) {
                body.push(Triangle(std::array::from_fn(|i| {
                    chunk.positions[usize::from(triangle[i])].into()
                })));
            }
        }
    }
    let body_surface = ghostline_refitter::surface::Surface::new(&body);
    for index in 0..layers.len() {
        if !layers[index].rest_space {
            continue;
        }
        let surfaces: Vec<_> = layers[..index]
            .iter()
            .filter(|other| {
                other.rest_space
                    && other.owner != layers[index].owner
                    && other.score > layers[index].score
            })
            .flat_map(|layer| triangles(layer, Some(&evaluated)))
            .collect();
        if surfaces.is_empty() && body.is_empty() {
            continue;
        }
        let directions = paths(&layers[index], native)?;
        let directions = layers[index]
            .chunks
            .iter()
            .zip(directions)
            .map(|(chunk, directions)| -> Result<Vec<SupportPath>> {
                let frame = evaluated.iter().find(|frame| {
                    frame.name == layers[index].name && frame.chunk == chunk.chunk_index
                });
                if let Some(frame) = frame {
                    anyhow::ensure!(
                        frame.transforms.len() == directions.len(),
                        "garment transform count differs from geometry"
                    );
                    Ok(directions
                        .into_iter()
                        .zip(&frame.transforms)
                        .map(|(path, transform)| {
                            let direction = transform.transform_vector3(path.direction);
                            let scale = if path.direction.length_squared() > 1e-12 {
                                direction.length() / path.direction.length()
                            } else {
                                0.
                            };
                            SupportPath {
                                direction,
                                maximum: path.maximum * scale,
                            }
                        })
                        .collect())
                } else {
                    Ok(directions
                        .into_iter()
                        .map(|_| SupportPath {
                            direction: Vec3::ZERO,
                            maximum: 0.,
                        })
                        .collect())
                }
            })
            .collect::<Result<Vec<_>>>()?;
        if directions.iter().flatten().all(|path| path.maximum <= 0.) {
            diagnostics.push(json!({"action":"layer fit has no authored support or skeleton guide","component":layers[index].name}));
            continue;
        }
        let mut hash = Sha256::new();
        hash.update(b"native-layers-v9/inline-support-morphs");
        for frame in evaluated
            .iter()
            .filter(|frame| frame.name == layers[index].name)
        {
            for transform in &frame.transforms {
                for value in transform.to_cols_array() {
                    hash.update(value.to_le_bytes());
                }
            }
        }
        hash.update(project.template.as_bytes());
        for index in &layers[index].overlays {
            hash.update(index.to_le_bytes());
        }
        let depth = layers
            .iter()
            .filter(|other| other.owner != layers[index].owner && other.score < layers[index].score)
            .map(|other| other.owner.as_str())
            .collect::<std::collections::BTreeSet<_>>()
            .len();
        let body_clearance = 0.002 * (1. + depth as f32);
        hash.update(body_clearance.to_le_bytes());
        for Triangle(triangle) in surfaces.iter().chain(&body) {
            for point in triangle {
                for value in point.to_array() {
                    hash.update(value.to_le_bytes());
                }
            }
        }
        for (chunk, paths) in layers[index].chunks.iter().zip(&directions) {
            for index in &chunk.indices {
                hash.update(index.to_le_bytes());
            }
            for point in &chunk.positions {
                for value in point {
                    hash.update(value.to_le_bytes());
                }
            }
            for path in paths {
                for value in path.direction.to_array() {
                    hash.update(value.to_le_bytes());
                }
                hash.update(path.maximum.to_le_bytes());
            }
        }
        let key = format!("{:x}", hash.finalize());
        let cached = FITS.lock().ok().and_then(|cache| cache.get(&key).cloned());
        let reused = cached.is_some();
        let mut changed = 0;
        let mut maximum = 0_f32;
        if let Some(fit) = cached {
            for (chunk, points) in layers[index].chunks.iter_mut().zip(fit.positions) {
                chunk.positions = points;
            }
            changed = fit.changed;
            maximum = fit.maximum;
        } else {
            let outer_surface = ghostline_refitter::surface::Surface::new(&surfaces);
            let attached_overlays = layers[index].attached.clone();
            let has_receiver = layers[index]
                .chunks
                .iter()
                .any(|c| !attached_overlays.contains(&c.chunk_index));
            let component_name = layers[index].name.clone();
            for (chunk, directions) in layers[index].chunks.iter_mut().zip(&directions) {
                if has_receiver && attached_overlays.contains(&chunk.chunk_index) {
                    continue;
                }
                let Some(frame) = evaluated
                    .iter()
                    .find(|frame| frame.name == component_name && frame.chunk == chunk.chunk_index)
                else {
                    continue;
                };
                let mut world = chunk
                    .positions
                    .iter()
                    .zip(&frame.transforms)
                    .map(|(point, transform)| {
                        transform.transform_point3((*point).into()).to_array()
                    })
                    .collect::<Vec<_>>();
                let report = fit_mesh_between(
                    &mut world,
                    &chunk.indices,
                    directions,
                    &body_surface,
                    &outer_surface,
                    [body_clearance, 0.006],
                );
                for ((point, fitted), transform) in
                    chunk.positions.iter_mut().zip(world).zip(&frame.transforms)
                {
                    let inverse = transform.inverse();
                    anyhow::ensure!(inverse.is_finite(), "singular garment fitting frame");
                    *point = inverse.transform_point3(fitted.into()).to_array();
                }
                changed += report.changed_vertices;
                maximum = maximum.max(report.maximum_displacement);
            }
            if let Ok(mut cache) = FITS.lock() {
                if cache.len() >= 16 {
                    cache.pop_first();
                }
                cache.insert(
                    key,
                    CachedFit {
                        positions: layers[index]
                            .chunks
                            .iter()
                            .map(|c| c.positions.clone())
                            .collect(),
                        changed,
                        maximum,
                    },
                );
            }
        }
        if changed > 0 {
            let layer = &mut layers[index];
            for chunk in &mut layer.chunks {
                let mut normals = vec![Vec3::ZERO; chunk.positions.len()];
                for triangle in chunk.indices.chunks_exact(3) {
                    let [a, b, c] = std::array::from_fn(|i| {
                        Vec3::from_array(chunk.positions[usize::from(triangle[i])])
                    });
                    let normal = (b - a).cross(c - a);
                    for &vertex in triangle {
                        normals[usize::from(vertex)] += normal;
                    }
                }
                for (index, normal) in normals.into_iter().enumerate() {
                    if normal.length_squared() > 1e-12 {
                        let normal = normal.normalize();
                        if let Some(value) = chunk.normals.get_mut(index) {
                            *value = normal.to_array();
                        }
                        if let Some(value) = chunk.tangents.get_mut(index) {
                            let tangent = Vec3::new(value[0], value[1], value[2]);
                            let tangent =
                                (tangent - normal * tangent.dot(normal)).normalize_or_zero();
                            value[0] = tangent.x;
                            value[1] = tangent.y;
                            value[2] = tangent.z;
                        }
                    }
                }
            }
            ghostline_red::mesh::replace_geometry(&mut layer.document, &layer.chunks)?;
            modified.insert(layer.mesh.clone(), layer.document.clone());
            diagnostics.push(json!({"action":"baked garment layer fit","algorithm":"native-layers-v9","space":"calibrated assembly","component":layer.name,"score":layer.score,"changed_vertices":changed,"maximum_displacement_m":maximum,"cache_reused":reused,"stage":"rest fit; pose coverage follows"}));
        }
    }
    for layer in &mut layers {
        overlays::attach(layer);
        if !layer.overlays.is_empty() {
            diagnostics.push(json!({"action":"attach native decal chunks to fitted garment surface","component":layer.name,"chunks":layer.overlays}));
        }
        update_shading(&mut layer.chunks);
        ghostline_red::mesh::replace_geometry(&mut layer.document, &layer.chunks)?;
        modified.insert(layer.mesh.clone(), layer.document.clone());
    }
    hide_covered_feet(native, &layers, source, diagnostics)?;
    visibility::mask_covered(native, &layers, source, modified, diagnostics)?;
    native_contacts::repair(native, &mut layers, source, modified, diagnostics)?;
    Ok(())
}

fn update_shading(chunks: &mut [MeshChunk]) {
    for chunk in chunks {
        let mut normals = vec![Vec3::ZERO; chunk.positions.len()];
        for triangle in chunk.indices.chunks_exact(3) {
            let [a, b, c] = std::array::from_fn(|i| {
                Vec3::from_array(chunk.positions[usize::from(triangle[i])])
            });
            for &vertex in triangle {
                normals[usize::from(vertex)] += (b - a).cross(c - a);
            }
        }
        for (index, normal) in normals.into_iter().enumerate() {
            if normal.length_squared() < 1e-12 {
                continue;
            }
            let normal = normal.normalize();
            if let Some(value) = chunk.normals.get_mut(index) {
                *value = normal.to_array();
            }
            if let Some(value) = chunk.tangents.get_mut(index) {
                let tangent = Vec3::new(value[0], value[1], value[2]);
                value[..3].copy_from_slice(
                    &(tangent - normal * tangent.dot(normal))
                        .normalize_or_zero()
                        .to_array(),
                );
            }
        }
    }
}

fn hide_covered_feet(
    native: &mut Assembly,
    layers: &[Layer],
    source: &GameSource,
    diagnostics: &mut Vec<Value>,
) -> Result<()> {
    let footwear: Vec<_> = layers
        .iter()
        .filter(|layer| {
            let name = layer.name.trim_start_matches(|c: char| c.is_ascii_digit());
            layer.rest_space && name.starts_with("s1_")
        })
        .flat_map(|layer| triangles(layer, None))
        .collect();
    if footwear.is_empty() {
        return Ok(());
    }
    let footwear_surface = ghostline_refitter::surface::Surface::new(&footwear);
    for component in &mut native.components {
        let Some(mesh) = &component.mesh else {
            continue;
        };
        let original = text(&component.template["name"]).unwrap_or(&component.name);
        if !original
            .split('_')
            .next()
            .is_some_and(|prefix| prefix.ends_with('0'))
        {
            continue;
        }
        let geometry = source.load_geometry(mesh)?;
        let Some(skin) = geometry.skin() else {
            continue;
        };
        for chunk in geometry.chunks() {
            if chunk.chunk_index >= 64 || chunk.skin.is_empty() {
                continue;
            }
            let is_foot = chunk.skin.iter().all(|vertex| {
                vertex
                    .joints
                    .iter()
                    .zip(vertex.weights)
                    .filter(|(_, weight)| *weight > 0.)
                    .filter_map(|(&joint, weight)| {
                        skin.names
                            .get(usize::from(joint))
                            .map(|name| (name, weight))
                    })
                    .filter(|(name, _)| name.contains("Foot") || name.contains("Toe"))
                    .map(|(_, weight)| weight)
                    .sum::<f32>()
                    > 0.6
            });
            if !is_foot {
                continue;
            }
            let mut anchors = Vec::new();
            for (index, name) in skin
                .names
                .iter()
                .enumerate()
                .filter(|(_, name)| name.contains("Toe"))
            {
                if chunk.skin.iter().any(|v| {
                    v.joints
                        .iter()
                        .zip(v.weights)
                        .any(|(&j, w)| usize::from(j) == index && w > 0.)
                }) {
                    anchors.push((name, skin.inverse_bind[index].inverse().w_axis.truncate()));
                }
            }
            if !anchors.is_empty()
                && anchors
                    .iter()
                    .all(|(_, point)| ghostline_refitter::layers::encloses(*point, &footwear))
            {
                component.chunk_mask &= !(1 << chunk.chunk_index);
                diagnostics.push(json!({"action":"hide body region enclosed by equipped footwear","component":component.name,"chunk":chunk.chunk_index,"native_bones":anchors.iter().map(|(name,_)|name).collect::<Vec<_>>()}));
            } else if !anchors.is_empty() {
                diagnostics.push(json!({"action":"footwear coverage assessment","component":component.name,"chunk":chunk.chunk_index,"anchors":anchors.iter().map(|(name,point)| {
                    let nearest = footwear_surface.closest(*point).map(|(surface, normal)| json!({"distance":point.distance(surface),"signed_distance":(*point-surface).dot(normal),"position":surface.to_array(),"normal":normal.to_array()}));
                    json!({"bone":name,"position":point.to_array(),"nearest":nearest})
                }).collect::<Vec<_>>()}));
            }
        }
    }
    Ok(())
}
