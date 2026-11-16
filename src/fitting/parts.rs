//! Equipment-slot ownership for disconnected parts of combined native garments.
use super::*;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Region {
    Upper,
    Lower,
    Unknown,
}
fn influence(name: &str) -> (f32, f32) {
    let name = name.to_ascii_lowercase();
    let lower = [
        "hips", "upleg", "leftleg", "rightleg", "thigh", "calf", "lowleg", "knee", "foot", "toe",
        "leg_",
    ]
    .iter()
    .any(|token| name.contains(token));
    let upper = [
        "spine",
        "chest",
        "shoulder",
        "arm",
        "hand",
        "neck",
        "scapula",
        "deltoid",
        "triceps",
        "biceps",
        "latissimus",
    ]
    .iter()
    .any(|token| name.contains(token));
    (f32::from(u8::from(upper)), f32::from(u8::from(lower)))
}
fn chest(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    [
        "spine2", "spine3", "chest", "shoulder", "arm", "neck", "scapula", "deltoid",
    ]
    .iter()
    .any(|token| name.contains(token))
}
fn leg_wrap(layer: &Layer, palette: &ghostline_red::mesh::MeshSkin) -> Option<[usize; 2]> {
    let bone = |name: &str| {
        palette
            .names
            .iter()
            .position(|bone| bone == name)
            .map(|i| palette.inverse_bind[i].inverse().w_axis.truncate())
    };
    let centers = [
        bone("LeftUpLeg")?.lerp(bone("LeftLeg")?, 0.35),
        bone("RightUpLeg")?.lerp(bone("RightLeg")?, 0.35),
    ];
    let axes = [
        (bone("LeftLeg")? - bone("LeftUpLeg")?).normalize_or_zero(),
        (bone("RightLeg")? - bone("RightUpLeg")?).normalize_or_zero(),
    ];
    let triangles:Vec<_>=layer.chunks.iter().filter(|chunk|!layer.overlays.contains(&chunk.chunk_index) && layer.document["Data"]["RootChunk"]["renderResourceBlob"]["Data"]["header"]["renderChunkInfos"][chunk.chunk_index]["lodMask"].as_u64()==Some(1))
        .flat_map(|chunk|chunk.indices.chunks_exact(3).map(|face|Triangle(std::array::from_fn(|i|Vec3::from_array(chunk.positions[usize::from(face[i])]))))).collect();
    let surface = ghostline_refitter::surface::Surface::new(&triangles);
    let radius = centers[0].distance(centers[1]) * 0.8;
    Some(std::array::from_fn(|i| {
        let right = axes[i].cross(Vec3::Z).normalize_or_zero();
        let forward = axes[i].cross(right).normalize_or_zero();
        (0..16)
            .filter(|&sample| {
                let angle = sample as f32 * std::f32::consts::TAU / 16.;
                surface
                    .distance(
                        centers[i],
                        right * angle.cos() + forward * angle.sin(),
                        true,
                    )
                    .is_some_and(|distance| distance < radius && distance > 0.015)
            })
            .count()
    }))
}
pub(super) fn resolve(
    project: &Project,
    layers: &mut [Layer],
    modified: &mut HashMap<ResourceKey, Value>,
    diagnostics: &mut Vec<Value>,
) -> Result<()> {
    let equipment = &project.appearance().equipment;
    for layer in layers {
        let Some(slot) = equipment
            .iter()
            .find_map(|(slot, id)| (id == &layer.owner).then_some(slot.as_str()))
        else {
            continue;
        };
        let replace = match slot {
            "Clo_InnerChest" | "Clo_OuterChest" if equipment.contains_key("Clo_Legs") => {
                Region::Lower
            }
            _ => continue,
        };
        let Some(palette) = ghostline_red::mesh::decode_skin(&layer.document)? else {
            continue;
        };
        let roles: Vec<_> = palette.names.iter().map(|name| influence(name)).collect();
        let mut regions = BTreeMap::new();
        let mut upper_faces = 0;
        let mut lower_faces = 0;
        let mut chest_area = 0.;
        let mut leg_area = 0.;
        let mut total_area = 0.;
        for chunk in &layer.chunks {
            if chunk.skin.len() != chunk.positions.len() {
                continue;
            }
            let faces: Vec<_> = chunk
                .indices
                .chunks_exact(3)
                .map(|face| std::array::from_fn(|i| usize::from(face[i])))
                .collect();
            let islands = ghostline_refitter::topology::connected_faces(&chunk.positions, &faces)
                .ok_or_else(|| anyhow::anyhow!("invalid garment topology"))?;
            let mut classified = vec![Region::Unknown; faces.len()];
            let mut face_votes = vec![(0., 0.); faces.len()];
            for (face, ids) in faces.iter().enumerate() {
                let [a, b, c] = ids.map(|i| Vec3::from_array(chunk.positions[i]));
                let area = (b - a).cross(c - a).length() / 3.;
                total_area += area * 3.;
                for &i in ids {
                    for (&joint, &weight) in chunk.skin[i].joints.iter().zip(&chunk.skin[i].weights)
                    {
                        if let Some(&(upper, lower)) = roles.get(usize::from(joint)) {
                            face_votes[face].0 += upper * weight;
                            face_votes[face].1 += lower * weight;
                            if chest(&palette.names[usize::from(joint)]) {
                                chest_area += weight * area;
                            }
                            if lower > 0. && palette.names[usize::from(joint)] != "Hips" {
                                leg_area += weight * area;
                            }
                        }
                    }
                }
            }
            for island in islands {
                let mut upper = 0.;
                let mut lower = 0.;
                let mut area = 0.;
                for &face in &island {
                    let ids = faces[face];
                    let [a, b, c] = ids.map(|i| Vec3::from_array(chunk.positions[i]));
                    let weight = (b - a).cross(c - a).length() / 3.;
                    for i in ids {
                        for (&joint, &skin_weight) in
                            chunk.skin[i].joints.iter().zip(&chunk.skin[i].weights)
                        {
                            if let Some(&(u, l)) = roles.get(usize::from(joint)) {
                                upper += u * skin_weight * weight;
                                lower += l * skin_weight * weight;
                            }
                        }
                    }
                    area += weight * 3.;
                }
                let region = if upper > area * 0.15 && lower < area * 0.05 {
                    Region::Upper
                } else if lower > area * 0.15 && upper < area * 0.05 {
                    Region::Lower
                } else {
                    Region::Unknown
                };
                for face in island {
                    classified[face] = if region == Region::Unknown {
                        let (upper, lower) = face_votes[face];
                        if upper + lower < 0.1 {
                            Region::Unknown
                        } else if upper >= lower {
                            Region::Upper
                        } else {
                            Region::Lower
                        }
                    } else {
                        region
                    };
                }
            }
            upper_faces += classified
                .iter()
                .filter(|&&region| region == Region::Upper)
                .count();
            lower_faces += classified
                .iter()
                .filter(|&&region| region == Region::Lower)
                .count();
            regions.insert(chunk.chunk_index, classified);
        }
        // A normal shirt/coat can extend below the pelvis. Split only a garment
        // with independent, positively identified upper AND leg surfaces.
        if upper_faces < 20
            || lower_faces < 20
            || chest_area < total_area * 0.05
            || leg_area < total_area * 0.10
        {
            continue;
        }
        let wrap = leg_wrap(layer, &palette).unwrap_or([0, 0]);
        diagnostics.push(json!({"action":"inspect combined garment anatomy","component":layer.name,"chest_area_fraction":chest_area/total_area,"leg_area_fraction":leg_area/total_area,"leg_wrap_samples":wrap}));
        if wrap.iter().any(|&count| count < 12) {
            continue;
        }
        let mut hidden = 0;
        for chunk in &mut layer.chunks {
            let Some(regions) = regions.get(&chunk.chunk_index) else {
                continue;
            };
            for (face, region) in chunk.indices.chunks_exact_mut(3).zip(regions) {
                if *region == replace {
                    face.fill(face[0]);
                    hidden += 1;
                }
            }
        }
        if hidden > 0 {
            ghostline_red::mesh::replace_indices(&mut layer.document, &layer.chunks)?;
            layer.original = layer.chunks.clone();
            modified.insert(layer.mesh.clone(), layer.document.clone());
            diagnostics.push(json!({"action":"replace separate part of combined garment","component":layer.name,"selected_slot":slot,"replaced_region":if replace==Region::Lower {"legs"}else{"torso"},"hidden_triangles":hidden,"upper_triangles":upper_faces,"lower_triangles":lower_faces}));
        }
    }
    Ok(())
}
