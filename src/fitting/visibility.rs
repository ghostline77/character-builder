//! Conservative opaque coverage shared by native preview and export.
use super::*;
use anyhow::Context;
use ghostline_renderer::{AssemblyOptions, EvaluatedChunk, SceneAssembly};
use rayon::prelude::*;

#[derive(Clone)]
struct Covered {
    triangles: Vec<bool>,
    boundary: Vec<Option<Vec3>>,
}
type Coverage = BTreeMap<(String, usize), Covered>;
#[derive(Clone, Copy)]
struct VertexConstraint {
    step: Vec3,
    normal: Vec3,
    gap: f32,
    point: Vec3,
    transform: glam::Mat4,
    body_covered: bool,
}
static COVERAGE: LazyLock<Mutex<BTreeMap<String, Coverage>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));

pub(super) fn body(name: &str) -> bool {
    ["t0_", "a0_", "l0_", "s0_"]
        .iter()
        .any(|prefix| name.contains(prefix))
}

pub(super) struct SkinSurface {
    surface: ghostline_refitter::surface::Surface,
    normals: BTreeMap<[u32; 3], Vec3>,
}

impl SkinSurface {
    pub(super) fn accepts(&self, point: Vec3) -> bool {
        !self.surface.contains(point)
            && self
                .surface
                .closest(point)
                .is_none_or(|(nearest, _)| nearest.distance(point) >= 0.001)
    }
    pub(super) fn new<'a>(chunks: impl IntoIterator<Item = &'a EvaluatedChunk>) -> Self {
        let mut faces = Vec::new();
        let mut normals = BTreeMap::<[u32; 3], Vec3>::new();
        for chunk in chunks {
            faces.extend(triangles(chunk));
            for (point, normal) in chunk.positions.iter().zip(&chunk.normals) {
                *normals.entry(point.map(f32::to_bits)).or_default() += Vec3::from_array(*normal);
            }
        }
        Self {
            surface: ghostline_refitter::surface::Surface::new(&faces),
            normals,
        }
    }

    pub(super) fn closest(&self, point: Vec3) -> Option<(Vec3, Vec3)> {
        let (nearest, Triangle([a, b, c])) = self.surface.closest_triangle(point)?;
        let ab = b - a;
        let ac = c - a;
        let ap = nearest - a;
        let denominator = ab.dot(ab) * ac.dot(ac) - ab.dot(ac).powi(2);
        if denominator.abs() < 1e-16 {
            return None;
        }
        let v = (ac.dot(ac) * ap.dot(ab) - ab.dot(ac) * ap.dot(ac)) / denominator;
        let w = (ab.dot(ab) * ap.dot(ac) - ab.dot(ac) * ap.dot(ab)) / denominator;
        let fallback = ab.cross(ac).normalize_or_zero();
        let normal = [a, b, c]
            .into_iter()
            .zip([1. - v - w, v, w])
            .map(|(vertex, weight)| {
                self.normals
                    .get(&vertex.to_array().map(f32::to_bits))
                    .copied()
                    .unwrap_or(fallback)
                    .normalize_or_zero()
                    * weight
            })
            .sum::<Vec3>()
            .normalize_or_zero();
        (normal != Vec3::ZERO).then_some((nearest, normal))
    }
}

pub(super) fn mask_covered(
    native: &Assembly,
    layers: &[Layer],
    source: &GameSource,
    modified: &mut HashMap<ResourceKey, Value>,
    diagnostics: &mut Vec<Value>,
) -> Result<()> {
    if layers.is_empty() {
        return Ok(());
    }
    let mut documents = HashMap::new();
    for component in &native.components {
        let Some(mesh) = &component.mesh else {
            continue;
        };
        if !body(&component.name) && !layers.iter().any(|layer| layer.name == component.name) {
            continue;
        }
        let document = modified
            .get(mesh)
            .cloned()
            .unwrap_or(source.resources().document(mesh)?.as_ref().clone());
        if document["Data"]["RootChunk"]["$type"] == "CMesh" {
            documents.insert(mesh.clone(), document);
        }
    }
    let mut hash = Sha256::new();
    hash.update(b"opaque-coverage-v9/calibrated-bind-space");
    hash.update(cache::key(native, layers, source)?.as_bytes());
    for layer in layers {
        for chunk in &layer.occluders {
            hash.update(chunk.to_le_bytes());
        }
        hash.update(layer.name.as_bytes());
    }
    let key = format!("{:x}", hash.finalize());
    let cached = COVERAGE
        .lock()
        .ok()
        .and_then(|cache| cache.get(&key).cloned());
    let reused = cached.is_some();
    let masks = if let Some(masks) = cached {
        masks
    } else {
        let assembly = SceneAssembly::from_native(
            native.clone(),
            &AssemblyOptions::default(),
            |resource, _| {
                let geometry = source.load_geometry(resource)?;
                let geometry = if let Some(document) = documents.get(resource) {
                    geometry.with_chunks(ghostline_red::mesh::decode_geometry(document, false)?)
                } else {
                    geometry
                };
                Ok(std::sync::Arc::new(geometry.with_neutral_material()?))
            },
        )?;
        let rig = *assembly
            .rigs
            .get("root")
            .context("coverage review requires locomotion rig")?;
        // Hidden skin still defines the anatomical guide beneath clothing.
        // Keep its rig frames, independently of the actual draw visibility.
        let mut reference = assembly.scene.clone();
        for chunk in assembly
            .scene
            .evaluated_geometry()?
            .iter()
            .filter(|chunk| body(&chunk.name))
        {
            reference.set_chunk_mask(chunk.node, u64::MAX)?;
        }
        let skeleton = assembly.scene.skeleton(rig)?.clone();
        let poses = crate::poses::CHECKS
            .par_iter()
            .map(|pose| -> Result<_> {
                let mut scene = assembly.scene.clone();
                scene.set_pose(rig, &pose.sample(&skeleton)?)?;
                let evaluated = scene.evaluated_geometry()?;
                let mut reference = reference.clone();
                reference.set_pose(rig, &pose.sample(&skeleton)?)?;
                let reference_geometry = reference.evaluated_geometry()?;
                let skin = SkinSurface::new(reference_geometry.iter().filter(|chunk| {
                    body(&chunk.name)
                        && native
                            .components
                            .iter()
                            .find(|component| component.name == chunk.name)
                            .and_then(|component| component.mesh.as_ref())
                            .and_then(|mesh| documents.get(mesh))
                            .is_none_or(|document| {
                                document["Data"]["RootChunk"]["renderResourceBlob"]["Data"]
                                            ["header"]["renderChunkInfos"][chunk.chunk]["lodMask"]
                                            .as_u64()
                                            == Some(1)
                            })
                }));
                let mut masks = BTreeMap::new();
                let mut surfaces = BTreeMap::new();
                for chunk in evaluated.iter().filter(|chunk| {
                    body(&chunk.name) || layers.iter().any(|layer| layer.name == chunk.name)
                }) {
                    let own = layers.iter().find(|layer| layer.name == chunk.name);
                    let key = own.map(|layer| (layer.owner.clone(), layer.score));
                    let surface = surfaces.entry(key).or_insert_with(|| {
                        let receivers: Vec<_> = evaluated
                            .iter()
                            .filter(|candidate| {
                                layers.iter().any(|layer| {
                                    layer.name == candidate.name
                                        && layer.occluders.contains(&candidate.chunk)
                                        && own.is_none_or(|own| {
                                            layer.owner != own.owner && layer.score > own.score
                                        })
                                        && layer.document["Data"]["RootChunk"]["renderResourceBlob"]
                                            ["Data"]["header"]["renderChunkInfos"][candidate.chunk]
                                            ["lodMask"]
                                            .as_u64()
                                            == Some(1)
                                })
                            })
                            .flat_map(triangles)
                            .collect();
                        ghostline_refitter::surface::Surface::new(&receivers)
                    });
                    let covered = chunk
                        .indices
                        .chunks_exact(3)
                        .map(|indices| {
                            let [a, b, c] = std::array::from_fn(|i| {
                                Vec3::from_array(chunk.positions[usize::from(indices[i])])
                            });
                            let normal = (b - a).cross(c - a).normalize_or_zero();
                            normal != Vec3::ZERO
                                && [
                                    [1., 0., 0.],
                                    [0., 1., 0.],
                                    [0., 0., 1.],
                                    [0.5, 0.5, 0.],
                                    [0., 0.5, 0.5],
                                    [0.5, 0., 0.5],
                                    [1. / 3.; 3],
                                ]
                                .into_iter()
                                .all(|weights| {
                                    let point = a * weights[0] + b * weights[1] + c * weights[2];
                                    let outward = skin
                                        .closest(point)
                                        .filter(|(nearest, _)| nearest.distance(point) < 0.1)
                                        .map_or(normal, |(_, normal)| normal);
                                    covered_point(point, outward, surface)
                                })
                        })
                        .collect::<Vec<_>>();
                    let mut boundary = chunk
                        .positions
                        .iter()
                        .enumerate()
                        .map(|(i, point)| {
                            own?;
                            let point = Vec3::from_array(*point);
                            let (skin_point, outward) = skin
                                .closest(point)
                                .filter(|(nearest, _)| nearest.distance(point) < 0.1)?;
                            let step = chunk.transforms[i]
                                .inverse()
                                .transform_vector3(contact_step(point, outward, surface));
                            let barrier =
                                chunk.transforms[i].transpose().transform_vector3(outward);
                            let gap = ((point - skin_point).dot(outward) - 0.003).max(0.);
                            Some(VertexConstraint {
                                step,
                                normal: barrier,
                                gap,
                                point,
                                transform: chunk.transforms[i],
                                body_covered: covered_point(point, outward, surface),
                            })
                        })
                        .collect::<Vec<Option<VertexConstraint>>>();
                    if own.is_some() {
                        for triangle in chunk.indices.chunks_exact(3) {
                            let ids: [usize; 3] = std::array::from_fn(|i| usize::from(triangle[i]));
                            for weights in
                                [[1. / 3.; 3], [0.5, 0.5, 0.], [0., 0.5, 0.5], [0.5, 0., 0.5]]
                            {
                                let point: Vec3 = (0..3)
                                    .map(|i| Vec3::from_array(chunk.positions[ids[i]]) * weights[i])
                                    .sum();
                                let Some((_, outward)) = skin
                                    .closest(point)
                                    .filter(|(nearest, _)| nearest.distance(point) < 0.1)
                                else {
                                    continue;
                                };
                                let world = contact_step(point, outward, surface);
                                if world.length_squared() < 1e-12 {
                                    continue;
                                }
                                let divisor =
                                    weights.iter().map(|weight| weight * weight).sum::<f32>();
                                for i in 0..3 {
                                    if let Some(observation) = &mut boundary[ids[i]] {
                                        let correction = chunk.transforms[ids[i]]
                                            .inverse()
                                            .transform_vector3(world)
                                            * (weights[i] / divisor);
                                        if correction.length_squared()
                                            > observation.step.length_squared()
                                        {
                                            observation.step = correction;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    masks.insert((chunk.name.clone(), chunk.chunk), (covered, boundary));
                }
                Ok((masks, skin))
            })
            .collect::<Result<Vec<_>>>()?;
        let Some((first, _)) = poses.first() else {
            return Ok(());
        };
        let mut masks = Coverage::new();
        for (key, (triangles, vertices)) in first {
            let mut triangles = triangles.clone();
            for (pose, _) in &poses[1..] {
                if let Some((other, _)) = pose.get(key) {
                    for (keep, value) in triangles.iter_mut().zip(other) {
                        *keep &= value;
                    }
                } else {
                    triangles.fill(false);
                }
            }
            let boundary = (0..vertices.len())
                .map(|i| {
                    let observations = poses
                        .iter()
                        .map(|(pose, skin)| {
                            Some((pose.get(key)?.1.get(i).cloned().flatten()?, skin))
                        })
                        .collect::<Option<Vec<_>>>()?;
                    let mut step = observations
                        .iter()
                        .map(|(observation, _)| observation.step)
                        .max_by(|a, b| a.length_squared().total_cmp(&b.length_squared()))?
                        .clamp_length_max(0.02);
                    let preferred = step;
                    for _ in 0..3 {
                        for (observation, _) in &observations {
                            let deficit = -observation.gap - step.dot(observation.normal);
                            if deficit > 0. && observation.normal.length_squared() > 1e-10 {
                                step += observation.normal
                                    * (deficit / observation.normal.length_squared());
                            }
                        }
                    }
                    if preferred.length_squared() > step.length_squared() + 1e-8 {
                        let accepts = |scale: f32| {
                            observations.iter().all(|(observation, skin)| {
                                let candidate = observation.point
                                    + observation.transform.transform_vector3(preferred * scale);
                                observation.body_covered || skin.accepts(candidate)
                            })
                        };
                        if accepts(0.) {
                            let mut low = 0.;
                            let mut high = 1.;
                            if accepts(1.) {
                                low = 1.;
                            } else {
                                for _ in 0..8 {
                                    let middle = (low + high) * 0.5;
                                    if accepts(middle) {
                                        low = middle;
                                    } else {
                                        high = middle;
                                    }
                                }
                            }
                            let candidate = preferred * low;
                            if candidate.length_squared() > step.length_squared() {
                                step = candidate;
                            }
                        }
                    }
                    (step.length_squared() > 1e-8).then_some(step)
                })
                .collect();
            masks.insert(
                key.clone(),
                Covered {
                    triangles,
                    boundary,
                },
            );
        }
        if let Ok(mut cache) = COVERAGE.lock() {
            if cache.len() >= 16 {
                cache.pop_first();
            }
            cache.insert(key, masks.clone());
        }
        masks
    };
    for (resource, mut document) in documents {
        let names: Vec<_> = native
            .components
            .iter()
            .filter(|c| c.mesh.as_ref() == Some(&resource))
            .map(|c| &c.name)
            .collect();
        let mut chunks = ghostline_red::mesh::decode_geometry(&document, false)?;
        let original = chunks.clone();
        let mut hidden = 0;
        let mut moved = 0;
        for chunk in &mut chunks {
            let mut visible = vec![false; chunk.positions.len()];
            for (i, triangle) in chunk.indices.chunks_exact_mut(3).enumerate() {
                // A shared mesh can have multiple instances; retain a triangle
                // if any instance or checked pose needs it to remain visible.
                if names.iter().all(|name| {
                    masks
                        .get(&((*name).clone(), chunk.chunk_index))
                        .and_then(|mask| mask.triangles.get(i))
                        .copied()
                        .unwrap_or(false)
                }) {
                    triangle.fill(triangle[0]);
                    hidden += 1;
                } else {
                    for index in triangle {
                        visible[usize::from(*index)] = true;
                    }
                }
            }
            if names.len() == 1
                && let Some(coverage) = masks.get(&(names[0].clone(), chunk.chunk_index))
            {
                for (i, point) in chunk.positions.iter_mut().enumerate() {
                    if visible[i]
                        && let Some(Some(step)) = coverage.boundary.get(i)
                    {
                        *point = (Vec3::from_array(*point) + *step).to_array();
                        moved += 1;
                    }
                }
            }
        }
        if let Some(layer) = layers.iter().find(|layer| layer.mesh == resource) {
            hidden += overlays::mask_attached(
                &document,
                &original,
                &mut chunks,
                &layer.overlays,
                &layer.attached,
            );
        }
        if moved > 0 {
            if let Some(layer) = layers.iter().find(|layer| layer.mesh == resource) {
                overlays::follow_delta(&original, &mut chunks, &layer.overlays, &layer.attached);
            }
            update_shading(&mut chunks);
            ghostline_red::mesh::replace_geometry(&mut document, &chunks)?;
        }
        if hidden > 0 || moved > 0 {
            ghostline_red::mesh::replace_indices(&mut document, &chunks)?;
            modified.insert(resource.clone(), document);
            diagnostics.push(json!({"action":"bake opaque clothing coverage masks across checked poses","resource":resource,"hidden_triangles":hidden,"boundary_vertices":moved,"poses":crate::poses::CHECKS,"cache_reused":reused}));
        }
    }
    Ok(())
}

fn triangles(chunk: &EvaluatedChunk) -> Vec<Triangle> {
    chunk
        .indices
        .chunks_exact(3)
        .filter(|indices| {
            indices[0] != indices[1] && indices[1] != indices[2] && indices[0] != indices[2]
        })
        .map(|indices| {
            Triangle(std::array::from_fn(|i| {
                Vec3::from_array(chunk.positions[usize::from(indices[i])])
            }))
        })
        .collect()
}

fn covered_point(
    point: Vec3,
    normal: Vec3,
    surface: &ghostline_refitter::surface::Surface,
) -> bool {
    surface
        .distance(point - normal * 0.08, normal, true)
        .is_some_and(|distance| distance < 0.16)
}

fn contact_step(
    point: Vec3,
    anatomical: Vec3,
    surface: &ghostline_refitter::surface::Surface,
) -> Vec3 {
    surface
        .distance(point - anatomical * 0.08, anatomical, true)
        .filter(|distance| *distance < 0.082)
        .map_or(Vec3::ZERO, |distance| anatomical * (distance - 0.082))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn anatomical_guide_uses_native_smooth_normals() {
        let points = [Vec3::ZERO, Vec3::X, Vec3::Y];
        let normal = Vec3::new(0.6, 0., 0.8);
        let skin = SkinSurface {
            surface: ghostline_refitter::surface::Surface::new(&[Triangle(points)]),
            normals: points
                .into_iter()
                .map(|point| (point.to_array().map(f32::to_bits), normal))
                .collect(),
        };
        let (_, actual) = skin.closest(Vec3::new(0.2, 0.3, 0.1)).unwrap();
        assert!(actual.distance(normal) < 1e-6);
    }

    #[test]
    fn opaque_coverage_keeps_open_edges_and_opposing_regions() {
        let surface = ghostline_refitter::surface::Surface::new(&[Triangle([
            Vec3::new(0., -1., -1.),
            Vec3::new(0., 1., -1.),
            Vec3::new(0., 0., 1.),
        ])]);
        assert!(covered_point(Vec3::new(-0.005, 0., 0.), Vec3::X, &surface));
        assert!(covered_point(Vec3::new(0.005, 0., 0.), Vec3::X, &surface));
        assert!(!covered_point(
            Vec3::new(-0.005, 1.01, -1.),
            Vec3::X,
            &surface
        ));
        assert!(!covered_point(
            Vec3::new(-0.005, 0., 0.),
            -Vec3::X,
            &surface
        ));
        assert!(!covered_point(Vec3::new(-0.1, 0., 0.), Vec3::X, &surface));
    }
}
