//! Native decal classification and attachment to the garment's deformation field.
use super::*;

pub(super) fn classify(
    source: &GameSource,
    document: &Value,
    appearance: &str,
    chunks: &[MeshChunk],
) -> Result<(
    std::collections::BTreeSet<usize>,
    std::collections::BTreeSet<usize>,
)> {
    let root = &document["Data"]["RootChunk"];
    let local = ghostline_red::material::local_material_documents(document, source.schema())?;
    let mut decal_names = std::collections::BTreeSet::new();
    let mut opaque_names = std::collections::BTreeSet::new();
    for entry in root["materialEntries"].as_array().into_iter().flatten() {
        let index = entry["index"].as_u64().unwrap_or(0) as usize;
        let mut material = if entry["isLocalInstance"] == true {
            local.get(index).cloned().unwrap_or(Value::Null)
        } else {
            let resource = root["externalMaterials"]
                .get(index)
                .or_else(|| root["preloadExternalMaterials"].get(index));
            resource
                .and_then(|r| ResourceKey::from_value(r).transpose())
                .transpose()?
                .map(|key| {
                    source
                        .resources()
                        .document(&key)
                        .map(|d| d.as_ref().clone())
                })
                .transpose()?
                .unwrap_or(Value::Null)
        };
        for _ in 0..24 {
            if material["Data"]["RootChunk"]["enableMask"] == true
                && let Some(name) = text(&entry["name"])
            {
                decal_names.insert(name.to_owned());
            }
            let Some(base) =
                ResourceKey::from_value(&material["Data"]["RootChunk"]["baseMaterial"])?
            else {
                break;
            };
            if matches!(&base,ResourceKey::Path(path) if path.contains("decal") && path.ends_with(".mt"))
            {
                if let Some(name) = text(&entry["name"]) {
                    decal_names.insert(name.to_owned());
                }
                break;
            }
            if matches!(&base,ResourceKey::Path(path) if path.ends_with(".mt")) {
                if matches!(&base,ResourceKey::Path(path) if path.ends_with("multilayered.mt"))
                    && let Some(name) = text(&entry["name"])
                    && !decal_names.contains(name)
                {
                    opaque_names.insert(name.to_owned());
                }
                break;
            }
            material = source.resources().document(&base)?.as_ref().clone();
        }
    }
    let names = root["appearances"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|a| a.get("Data").unwrap_or(a))
        .find(|a| text(&a["name"]).unwrap_or("default") == appearance)
        .and_then(|a| a["chunkMaterials"].as_array())
        .into_iter()
        .flatten()
        .filter_map(text)
        .collect::<Vec<_>>();
    let overlays = chunks
        .iter()
        .filter(|c| !names.is_empty() && decal_names.contains(names[c.chunk_index % names.len()]))
        .map(|c| c.chunk_index)
        .collect();
    let opaque = chunks
        .iter()
        .filter(|c| !names.is_empty() && opaque_names.contains(names[c.chunk_index % names.len()]))
        .map(|c| c.chunk_index)
        .collect();
    Ok((overlays, opaque))
}

pub(super) fn receivers(
    chunks: &[MeshChunk],
    overlays: &std::collections::BTreeSet<usize>,
) -> std::collections::BTreeSet<usize> {
    let triangles: Vec<_> = chunks
        .iter()
        .filter(|c| !overlays.contains(&c.chunk_index))
        .flat_map(|c| {
            c.indices.chunks_exact(3).map(|t| {
                Triangle(std::array::from_fn(|i| {
                    Vec3::from_array(c.positions[usize::from(t[i])])
                }))
            })
        })
        .collect();
    let surface = ghostline_refitter::surface::Surface::new(&triangles);
    chunks
        .iter()
        .filter(|c| overlays.contains(&c.chunk_index))
        .filter(|chunk| {
            let sample: Vec<_> = chunk
                .positions
                .iter()
                .step_by((chunk.positions.len() / 200).max(1))
                .collect();
            let close = sample
                .iter()
                .filter(|point| {
                    surface
                        .closest(Vec3::from_array(***point))
                        .is_some_and(|(nearest, _)| {
                            nearest.distance(Vec3::from_array(***point)) < 0.006
                        })
                })
                .count();
            !sample.is_empty() && close * 4 >= sample.len() * 3
        })
        .map(|c| c.chunk_index)
        .collect()
}

pub(super) fn attach(layer: &mut Layer) {
    follow_delta(
        &layer.original,
        &mut layer.chunks,
        &layer.overlays,
        &layer.attached,
    );
}

/// Attached cards inherit the visibility of their opaque cloth receiver.
pub(super) fn mask_attached(
    document: &Value,
    original: &[MeshChunk],
    chunks: &mut [MeshChunk],
    overlays: &std::collections::BTreeSet<usize>,
    attached: &std::collections::BTreeSet<usize>,
) -> usize {
    type Key = [[u32; 3]; 3];
    fn key(Triangle(points): Triangle) -> Key {
        let mut points = points.map(|point| point.to_array().map(f32::to_bits));
        points.sort_unstable();
        points
    }
    let infos =
        &document["Data"]["RootChunk"]["renderResourceBlob"]["Data"]["header"]["renderChunkInfos"];
    let mut lods = BTreeMap::<u64, (Vec<Triangle>, std::collections::BTreeSet<Key>)>::new();
    for source in original
        .iter()
        .filter(|chunk| !overlays.contains(&chunk.chunk_index))
    {
        let Some(fitted) = chunks
            .iter()
            .find(|chunk| chunk.chunk_index == source.chunk_index)
        else {
            continue;
        };
        let lod = infos[source.chunk_index]["lodMask"].as_u64().unwrap_or(0);
        let (triangles, hidden) = lods.entry(lod).or_default();
        for (a, b) in source
            .indices
            .chunks_exact(3)
            .zip(fitted.indices.chunks_exact(3))
        {
            let triangle = Triangle(std::array::from_fn(|i| {
                Vec3::from_array(source.positions[usize::from(a[i])])
            }));
            if b[0] == b[1] && b[1] == b[2] {
                hidden.insert(key(triangle));
            }
            triangles.push(triangle);
        }
    }
    let lods: BTreeMap<_, _> = lods
        .into_iter()
        .map(|(lod, (triangles, hidden))| {
            (
                lod,
                (
                    ghostline_refitter::surface::Surface::new(&triangles),
                    hidden,
                ),
            )
        })
        .collect();
    let mut count = 0;
    for chunk in chunks
        .iter_mut()
        .filter(|chunk| attached.contains(&chunk.chunk_index))
    {
        let Some(source) = original
            .iter()
            .find(|source| source.chunk_index == chunk.chunk_index)
        else {
            continue;
        };
        let lod = infos[chunk.chunk_index]["lodMask"].as_u64().unwrap_or(0);
        let Some((surface, hidden)) = lods.get(&lod) else {
            continue;
        };
        for (a, b) in source
            .indices
            .chunks_exact(3)
            .zip(chunk.indices.chunks_exact_mut(3))
        {
            if b[0] == b[1] && b[1] == b[2] {
                continue;
            }
            let point = a
                .iter()
                .map(|index| Vec3::from_array(source.positions[usize::from(*index)]))
                .sum::<Vec3>()
                / 3.;
            if surface
                .closest_triangle(point)
                .is_some_and(|(_, triangle)| hidden.contains(&key(triangle)))
            {
                b.fill(b[0]);
                count += 1;
            }
        }
    }
    count
}

pub(super) fn follow_delta(
    original: &[MeshChunk],
    chunks: &mut [MeshChunk],
    overlays: &std::collections::BTreeSet<usize>,
    attached: &std::collections::BTreeSet<usize>,
) {
    if overlays.is_empty() {
        return;
    }
    let solids: Vec<_> = original
        .iter()
        .filter(|c| !overlays.contains(&c.chunk_index))
        .collect();
    if solids.is_empty() {
        return;
    }
    let triangles: Vec<_> = solids
        .iter()
        .flat_map(|c| {
            c.indices
                .chunks_exact(3)
                .filter(|t| t[0] != t[1] && t[1] != t[2] && t[0] != t[2])
                .map(|t| {
                    Triangle(std::array::from_fn(|i| {
                        Vec3::from_array(c.positions[usize::from(t[i])])
                    }))
                })
        })
        .collect();
    let receiver = ghostline_refitter::surface::Surface::new(&triangles);
    let mut deltas: BTreeMap<[u32; 3], (Vec3, usize)> = BTreeMap::new();
    for original in solids {
        let Some(fitted) = chunks
            .iter()
            .find(|c| c.chunk_index == original.chunk_index)
        else {
            continue;
        };
        for (old, new) in original.positions.iter().zip(&fitted.positions) {
            let value = deltas
                .entry(old.map(f32::to_bits))
                .or_insert((Vec3::ZERO, 0));
            value.0 += Vec3::from_array(*new) - Vec3::from_array(*old);
            value.1 += 1;
        }
    }
    for chunk in chunks
        .iter_mut()
        .filter(|c| attached.contains(&c.chunk_index))
    {
        let Some(original) = original.iter().find(|c| c.chunk_index == chunk.chunk_index) else {
            continue;
        };
        for (point, old) in chunk.positions.iter_mut().zip(&original.positions) {
            let origin = Vec3::from_array(*old);
            let Some((nearest, Triangle([a, b, c]))) = receiver.closest_triangle(origin) else {
                continue;
            };
            let ab = b - a;
            let ac = c - a;
            let ap = nearest - a;
            let denominator = ab.dot(ab) * ac.dot(ac) - ab.dot(ac) * ab.dot(ac);
            if denominator.abs() < 1e-16 {
                continue;
            }
            let v = (ac.dot(ac) * ap.dot(ab) - ab.dot(ac) * ap.dot(ac)) / denominator;
            let w = (ab.dot(ab) * ap.dot(ac) - ab.dot(ac) * ap.dot(ab)) / denominator;
            let delta = [a, b, c]
                .into_iter()
                .zip([1. - v - w, v, w])
                .map(|(p, weight)| {
                    deltas
                        .get(&p.to_array().map(f32::to_bits))
                        .map_or(Vec3::ZERO, |(delta, count)| {
                            *delta * weight / (*count as f32)
                        })
                })
                .sum::<Vec3>();
            *point = (origin + delta).to_array();
        }
    }
}
