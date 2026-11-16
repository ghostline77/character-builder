//! Geometry and rig identity shared by the pose-coverage cache.
use super::*;

pub(super) fn key(native: &Assembly, layers: &[Layer], source: &GameSource) -> Result<String> {
    let mut hash = Sha256::new();
    hash.update(b"native-fit-coverage-v1");
    for component in &native.components {
        hash.update(component.name.as_bytes());
        hash.update(component.parent.component.as_bytes());
        hash.update(component.control.component.as_bytes());
        hash.update(component.chunk_mask.to_le_bytes());
        hash.update([u8::from(component.enabled)]);
        for value in component.local.to_cols_array() {
            hash.update(value.to_le_bytes());
        }
        if let Some(mesh) = &component.mesh {
            hash.update(source.load_geometry(mesh)?.source_sha256.as_bytes());
        }
    }
    let mut rigs: Vec<_> = native.rigs.iter().collect();
    rigs.sort_by_key(|(key, _)| key.native_hash());
    for (key, rig) in rigs {
        hash.update(key.native_hash().to_le_bytes());
        for bone in &rig.bones {
            hash.update(bone.name.as_bytes());
            hash.update(bone.parent.unwrap_or(usize::MAX).to_le_bytes());
            for value in bone.rest.matrix()?.to_cols_array() {
                hash.update(value.to_le_bytes());
            }
        }
    }
    for layer in layers {
        hash.update(layer.score.to_le_bytes());
        for index in &layer.overlays {
            hash.update(index.to_le_bytes());
        }
        for chunk in &layer.chunks {
            for skin in &chunk.skin {
                for (&joint, weight) in skin.joints.iter().zip(skin.weights) {
                    hash.update(joint.to_le_bytes());
                    hash.update(weight.to_le_bytes());
                }
            }
            for point in &chunk.positions {
                for value in point {
                    hash.update(value.to_le_bytes());
                }
            }
            for index in &chunk.indices {
                hash.update(index.to_le_bytes());
            }
        }
    }
    Ok(format!("{:x}", hash.finalize()))
}
