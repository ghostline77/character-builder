//! Conservative intersection masks along authored garment support directions.
//! No garment positions or skinning streams are changed by this pass.
use super::*;
use rayon::prelude::*;
use std::collections::BTreeSet;

type TriangleKey = [[u32; 3]; 3];
type Masks = BTreeMap<(String, usize), Vec<bool>>;
static MASKS: LazyLock<Mutex<BTreeMap<String, Masks>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));
fn key(Triangle(points): Triangle) -> TriangleKey {
    points.map(|point| point.to_array().map(f32::to_bits))
}

fn barycentric(point: Vec3, Triangle([a, b, c]): Triangle) -> Option<[f32; 3]> {
    let ab = b - a;
    let ac = c - a;
    let ap = point - a;
    let denominator = ab.dot(ab) * ac.dot(ac) - ab.dot(ac).powi(2);
    if denominator.abs() < 1e-16 {
        return None;
    }
    let v = (ac.dot(ac) * ap.dot(ab) - ab.dot(ac) * ap.dot(ac)) / denominator;
    let w = (ab.dot(ab) * ap.dot(ac) - ab.dot(ac) * ap.dot(ab)) / denominator;
    Some([1. - v - w, v, w])
}

struct Attributes {
    normals: [Vec3; 3],
    caps: [bool; 3],
}
struct Receiver {
    surface: ghostline_refitter::surface::Surface,
    attributes: BTreeMap<TriangleKey, Attributes>,
}
impl Receiver {
    fn new(layers: &[Layer], own: &Layer, frames: &[EvaluatedChunk]) -> Self {
        let mut triangles = Vec::new();
        let mut attributes = BTreeMap::new();
        for layer in layers
            .iter()
            .filter(|layer| layer.owner != own.owner && layer.score > own.score)
        {
            let caps = layer.document["Data"]["RootChunk"]["parameters"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|p| p.get("Data").unwrap_or(p))
                .find(|p| p["$type"] == "meshMeshParamGarmentSupport");
            for chunk in layer.chunks.iter().filter(|chunk| layer.occluders.contains(&chunk.chunk_index)
                && layer.document["Data"]["RootChunk"]["renderResourceBlob"]["Data"]["header"]["renderChunkInfos"][chunk.chunk_index]["lodMask"].as_u64() == Some(1)) {
                let Some(frame) = frames.iter().find(|frame| frame.name == layer.name && frame.chunk == chunk.chunk_index) else { continue; };
                let cap: BTreeSet<_> = caps.and_then(|c| c["chunkCapVertices"][chunk.chunk_index].as_array())
                    .into_iter().flatten().filter_map(Value::as_u64).map(|i| i as usize).collect();
                for face in chunk.indices.chunks_exact(3).filter(|f| f[0] != f[1] && f[1] != f[2] && f[0] != f[2]) {
                    let ids: [usize; 3] = std::array::from_fn(|i| usize::from(face[i]));
                    let triangle = Triangle(ids.map(|i| Vec3::from_array(frame.positions[i])));
                    attributes.insert(key(triangle), Attributes {
                        normals: ids.map(|i| Vec3::from_array(frame.normals[i])),
                        caps: ids.map(|i| cap.contains(&i)),
                    });
                    triangles.push(triangle);
                }
            }
        }
        Self {
            surface: ghostline_refitter::surface::Surface::new(&triangles),
            attributes,
        }
    }

    fn cap(&self, point: Vec3, triangle: Triangle) -> bool {
        let Some(weights) = barycentric(point, triangle) else {
            return true;
        };
        let Some(attributes) = self.attributes.get(&key(triangle)) else {
            return true;
        };
        (0..3)
            .map(|i| if attributes.caps[i] { weights[i] } else { 0. })
            .sum::<f32>()
            >= 0.5
    }

    fn covers(&self, point: Vec3, outward: Vec3) -> bool {
        if outward.length_squared() < 1e-10 {
            return false;
        }
        let outward = outward.normalize();
        let mut origin = point - outward * 0.08;
        let end = point + outward * 0.08;
        // Ignore the reverse face of a fold, but never cross a protected cap.
        for _ in 0..8 {
            let Some(distance) = self.surface.distance(origin, outward, true) else {
                return false;
            };
            let hit = origin + outward * distance;
            if (end - hit).dot(outward) < 0. {
                return false;
            }
            let Some((_, triangle)) = self.surface.closest_triangle(hit) else {
                return false;
            };
            if self.cap(hit, triangle) {
                return false;
            }
            let Some(weights) = barycentric(hit, triangle) else {
                return false;
            };
            let attributes = &self.attributes[&key(triangle)];
            let normal = (0..3)
                .map(|i| attributes.normals[i] * weights[i])
                .sum::<Vec3>()
                .normalize_or_zero();
            if normal.dot(outward) > 0. {
                return true;
            }
            origin = hit + outward * 0.0001;
        }
        false
    }

    fn covers_nearest(&self, point: Vec3) -> bool {
        let Some((nearest, Triangle([a, b, c]))) = self.surface.closest_triangle(point) else {
            return false;
        };
        nearest.distance_squared(point) < 0.08_f32.powi(2)
            && self.covers(point, (b - a).cross(c - a).normalize_or_zero())
    }

    fn intersects(&self, triangle: Triangle) -> bool {
        self.surface
            .crossings_triangle(triangle)
            .into_iter()
            .any(|crossing| {
                !self.cap(
                    (crossing.segment[0] + crossing.segment[1]) * 0.5,
                    crossing.triangle,
                )
            })
    }
}

pub(super) fn repair(
    native: &Assembly,
    layers: &mut [Layer],
    source: &GameSource,
    modified: &mut HashMap<ResourceKey, Value>,
    diagnostics: &mut Vec<Value>,
) -> Result<()> {
    for layer in layers.iter_mut() {
        if let Some(document) = modified.get(&layer.mesh) {
            layer.document = document.clone();
            layer.chunks = ghostline_red::mesh::decode_geometry(document, false)?;
        }
    }
    let mut hash = Sha256::new();
    hash.update(b"native-intersection-coverage-v1");
    hash.update(cache::key(native, layers, source)?.as_bytes());
    for layer in layers.iter() {
        for index in &layer.occluders {
            hash.update(index.to_le_bytes());
        }
    }
    let key = format!("{:x}", hash.finalize());
    let cached = MASKS.lock().ok().and_then(|cache| cache.get(&key).cloned());
    let masks = if let Some(masks) = cached {
        masks
    } else {
        let masks = coverage(native, layers, source, modified, diagnostics)?;
        if let Ok(mut cache) = MASKS.lock() {
            if cache.len() >= 16 {
                cache.pop_first();
            }
            cache.insert(key, masks.clone());
        }
        masks
    };
    apply_masks(native, layers, modified, diagnostics, &masks)
}

fn coverage(
    native: &Assembly,
    layers: &[Layer],
    source: &GameSource,
    modified: &HashMap<ResourceKey, Value>,
    diagnostics: &mut Vec<Value>,
) -> Result<Masks> {
    let assembly = SceneAssembly::from_native(
        native.clone(),
        &AssemblyOptions::default(),
        |resource, _| {
            let geometry = source.load_geometry(resource)?;
            let geometry = if let Some(document) = modified.get(resource) {
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
        .ok_or_else(|| anyhow::anyhow!("garment coverage requires locomotion rig"))?;
    let skeleton = assembly.scene.skeleton(rig)?.clone();
    let frames = crate::poses::CHECKS
        .par_iter()
        .map(|pose| -> Result<_> {
            let mut scene = assembly.scene.clone();
            scene.set_pose(rig, &pose.sample(&skeleton)?)?;
            Ok(scene.evaluated_geometry()?)
        })
        .collect::<Result<Vec<_>>>()?;
    let mut masks = BTreeMap::new();
    for layer in layers.iter() {
        if !layers
            .iter()
            .any(|other| other.owner != layer.owner && other.score > layer.score)
        {
            continue;
        }
        let receivers: Vec<_> = frames
            .iter()
            .map(|frame| Receiver::new(layers, layer, frame))
            .collect();
        let morphs = ghostline_red::mesh::decode_garment_morphs(&layer.document)?;
        let palette = ghostline_red::mesh::decode_skin(&layer.document)?;
        for chunk in layer
            .chunks
            .iter()
            .filter(|c| !layer.overlays.contains(&c.chunk_index))
        {
            let inline = morphs.iter().find(|m| m.chunk_index == chunk.chunk_index);
            let directions: Vec<_> = chunk
                .positions
                .iter()
                .enumerate()
                .map(|(i, &point)| {
                    if let Some(morph) = inline {
                        return -morph.offsets[i].normalize_or_zero();
                    }
                    palette
                        .as_ref()
                        .zip(chunk.skin.get(i))
                        .map_or(Vec3::ZERO, |(palette, skin)| {
                            let center: Vec3 = skin
                                .joints
                                .iter()
                                .zip(&skin.weights)
                                .filter(|(_, w)| **w > 0.)
                                .filter_map(|(&j, &w)| {
                                    palette
                                        .inverse_bind
                                        .get(usize::from(j))
                                        .map(|matrix| matrix.inverse().w_axis.truncate() * w)
                                })
                                .sum();
                            (Vec3::from_array(point) - center).normalize_or_zero()
                        })
                })
                .collect();
            let observations = frames
                .par_iter()
                .zip(&receivers)
                .map(|(pose, receiver)| {
                    let Some(frame) = pose
                        .iter()
                        .find(|f| f.name == layer.name && f.chunk == chunk.chunk_index)
                    else {
                        return vec![(false, false); chunk.indices.len() / 3];
                    };
                    chunk
                        .indices
                        .chunks_exact(3)
                        .map(|face| {
                            if face[0] == face[1] || face[1] == face[2] || face[0] == face[2] {
                                return (false, false);
                            }
                            let ids: [usize; 3] = std::array::from_fn(|i| usize::from(face[i]));
                            let points = ids.map(|i| Vec3::from_array(frame.positions[i]));
                            let guides =
                                ids.map(|i| frame.transforms[i].transform_vector3(directions[i]));
                            let normals = ids.map(|i| Vec3::from_array(frame.normals[i]));
                            let covered = [
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
                                let point = (0..3).map(|i| points[i] * weights[i]).sum();
                                receiver.covers(point, (0..3).map(|i| guides[i] * weights[i]).sum())
                                    || receiver.covers(
                                        point,
                                        (0..3).map(|i| normals[i] * weights[i]).sum(),
                                    )
                                    || receiver.covers_nearest(point)
                            });
                            (covered, receiver.intersects(Triangle(points)))
                        })
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();
            if let Ok(trace) = std::env::var("GHOSTLINE_TRACE_TRIANGLE") {
                for face in trace
                    .split(',')
                    .filter_map(|s| s.parse::<usize>().ok())
                    .filter(|&i| i < chunk.indices.len() / 3)
                {
                    let ids: [usize; 3] =
                        std::array::from_fn(|i| usize::from(chunk.indices[face * 3 + i]));
                    let samples: Vec<_> = frames.iter().zip(&receivers).map(|(pose, receiver)| {
                        let Some(frame) = pose.iter().find(|f| f.name==layer.name && f.chunk==chunk.chunk_index) else { return json!(null); };
                        let points = ids.map(|i| Vec3::from_array(frame.positions[i]));
                        let guides = ids.map(|i| frame.transforms[i].transform_vector3(directions[i]));
                        let samples: Vec<_> = [[1.,0.,0.],[0.,1.,0.],[0.,0.,1.],[0.5,0.5,0.],[0.,0.5,0.5],[0.5,0.,0.5],[1./3.;3]].into_iter().map(|w| {
                            let p: Vec3=(0..3).map(|i|points[i]*w[i]).sum();
                            let d: Vec3=(0..3).map(|i|guides[i]*w[i]).sum::<Vec3>().normalize_or_zero();
                            json!({"covered":receiver.covers(p,d),"outward":d.to_array(),"exit":receiver.surface.distance(p-d*0.08,d,true)})
                        }).collect();
                        json!(samples)
                    }).collect();
                    diagnostics.push(json!({"action":"trace native coverage","component":layer.name,"chunk":chunk.chunk_index,"triangle":face,"samples_by_pose":samples,"observations":observations.iter().map(|p|p[face]).collect::<Vec<_>>()}));
                }
            }
            let eligible: Vec<_> = (0..chunk.indices.len() / 3)
                .map(|i| observations.iter().all(|pose| pose[i].0))
                .collect();
            let faces: Vec<[usize; 3]> = chunk
                .indices
                .chunks_exact(3)
                .map(|f| std::array::from_fn(|i| usize::from(f[i])))
                .collect();
            let selected: Vec<_> = faces
                .iter()
                .zip(&eligible)
                .filter_map(|(face, &eligible)| eligible.then_some(*face))
                .collect();
            let original: Vec<_> = eligible
                .iter()
                .enumerate()
                .filter_map(|(i, &keep)| keep.then_some(i))
                .collect();
            let groups = ghostline_refitter::topology::connected_faces(&chunk.positions, &selected)
                .ok_or_else(|| anyhow::anyhow!("invalid garment coverage topology"))?;
            let mut hidden = vec![false; faces.len()];
            for group in groups {
                if group
                    .iter()
                    .any(|&i| observations.iter().any(|pose| pose[original[i]].1))
                {
                    for i in group {
                        hidden[original[i]] = true;
                    }
                }
            }
            diagnostics.push(json!({"action":"inspect native garment intersection coverage","component":layer.name,"chunk":chunk.chunk_index,"eligible_triangles":eligible.iter().filter(|v| **v).count(),"crossing_triangles":(0..faces.len()).filter(|&i|observations.iter().any(|pose|pose[i].1)).count(),"hidden_triangles":hidden.iter().filter(|v| **v).count()}));
            masks.insert((layer.name.clone(), chunk.chunk_index), hidden);
        }
    }
    Ok(masks)
}

fn apply_masks(
    native: &Assembly,
    layers: &mut [Layer],
    modified: &mut HashMap<ResourceKey, Value>,
    diagnostics: &mut Vec<Value>,
    masks: &Masks,
) -> Result<()> {
    for layer in layers.iter_mut() {
        let names: Vec<_> = native
            .components
            .iter()
            .filter(|c| c.mesh.as_ref() == Some(&layer.mesh))
            .map(|c| &c.name)
            .collect();
        let original = layer.chunks.clone();
        let mut hidden = 0;
        for chunk in &mut layer.chunks {
            for (i, face) in chunk.indices.chunks_exact_mut(3).enumerate() {
                if face[0] != face[1]
                    && names.iter().all(|name| {
                        masks
                            .get(&((*name).clone(), chunk.chunk_index))
                            .and_then(|mask| mask.get(i))
                            .copied()
                            .unwrap_or(false)
                    })
                {
                    face.fill(face[0]);
                    hidden += 1;
                }
            }
        }
        if hidden > 0 {
            hidden += overlays::mask_attached(
                &layer.document,
                &original,
                &mut layer.chunks,
                &layer.overlays,
                &layer.attached,
            );
            ghostline_red::mesh::replace_indices(&mut layer.document, &layer.chunks)?;
            modified.insert(layer.mesh.clone(), layer.document.clone());
            diagnostics.push(json!({"action":"mask intersecting underlayers with native garment support","component":layer.name,"hidden_triangles":hidden,"poses":crate::poses::CHECKS}));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coverage_preserves_caps_openings_and_opposite_surfaces() {
        let triangle = Triangle([
            Vec3::new(0., -1., -1.),
            Vec3::new(0., 1., -1.),
            Vec3::new(0., 0., 1.),
        ]);
        let mut receiver = Receiver {
            surface: ghostline_refitter::surface::Surface::new(&[triangle]),
            attributes: BTreeMap::from([(
                key(triangle),
                Attributes {
                    normals: [Vec3::X; 3],
                    caps: [false; 3],
                },
            )]),
        };
        assert!(receiver.covers(Vec3::new(0.03, 0., 0.), Vec3::X));
        assert!(receiver.covers_nearest(Vec3::new(0.03, 0., 0.)));
        assert!(!receiver.covers(Vec3::new(0.03, 2., 0.), Vec3::X));
        assert!(!receiver.covers_nearest(Vec3::new(0.03, 2., 0.)));
        assert!(!receiver.covers(Vec3::new(0.03, 0., 0.), -Vec3::X));
        assert!(!receiver.covers(Vec3::new(0.1, 0., 0.), Vec3::X));
        receiver.attributes.get_mut(&key(triangle)).unwrap().caps = [true; 3];
        assert!(!receiver.covers(Vec3::new(0.03, 0., 0.), Vec3::X));
        assert!(!receiver.covers_nearest(Vec3::new(0.03, 0., 0.)));
    }
}
