//! Deterministic joint stress poses, independent of particular outfits or rigs.
use anyhow::{Result, ensure};
use ghostline_renderer::{Pose, Skeleton};
use glam::{Mat4, Quat};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckPose {
    #[default]
    Rest,
    Relaxed,
    Reach,
    Stride,
    Crouch,
    Seated,
    Twist,
}
pub const CHECKS: &[CheckPose] = &[
    CheckPose::Rest,
    CheckPose::Relaxed,
    CheckPose::Reach,
    CheckPose::Stride,
    CheckPose::Crouch,
    CheckPose::Seated,
    CheckPose::Twist,
];

pub fn clothing(name: &str) -> bool {
    [
        "t1_", "t2_", "l1_", "s1_", "g1_", "i1_", "h1_", "h2_", "f1_",
    ]
    .iter()
    .any(|prefix| name.contains(prefix))
}

pub fn inspect(chunks: &[ghostline_renderer::EvaluatedChunk]) -> serde_json::Value {
    use ghostline_refitter::{Triangle, surface::Surface};
    use glam::Vec3;
    let body: Vec<_> = chunks
        .iter()
        .filter(|chunk| {
            ["t0_", "a0_", "l0_", "s0_", "h0_"]
                .iter()
                .any(|prefix| chunk.name.contains(prefix))
        })
        .flat_map(|chunk| {
            chunk.indices.chunks_exact(3).map(|t| {
                Triangle(std::array::from_fn(|i| {
                    Vec3::from_array(chunk.positions[usize::from(t[i])])
                }))
            })
        })
        .collect();
    let surface = Surface::new(&body);
    let rows:Vec<_>=chunks.iter().filter(|chunk|clothing(&chunk.name)).map(|chunk| {
        let mut deepest=0_f32;
        let mut vertices=Vec::new();
        for (index,point) in chunk.positions.iter().enumerate() {
            let point=Vec3::from_array(*point);
            if let Some((nearest,normal))=surface.closest(point) {
                let depth=-(point-nearest).dot(normal);
                if depth>0.003 && depth<0.08 { vertices.push(index);deepest=deepest.max(depth); }
            }
        }
        serde_json::json!({"component":chunk.name,"mesh":chunk.mesh,"chunk":chunk.chunk,"body_penetration_candidates":vertices.len(),"maximum_depth_m":deepest,"vertices":vertices})
    }).collect();
    serde_json::json!({"method":"posed visible skin surface; 3mm tolerance; candidates require visual review","garments":rows})
}
impl CheckPose {
    pub fn name(self) -> &'static str {
        match self {
            Self::Rest => "rest",
            Self::Relaxed => "relaxed",
            Self::Reach => "reach",
            Self::Stride => "stride",
            Self::Crouch => "crouch",
            Self::Seated => "seated",
            Self::Twist => "twist",
        }
    }
    pub fn sample(self, skeleton: &Skeleton) -> Result<Pose> {
        let mut pose = skeleton.rest_pose().clone();
        let rx = |v: f32| Quat::from_rotation_x(v.to_radians());
        let ry = |v: f32| Quat::from_rotation_y(v.to_radians());
        let rz = |v: f32| Quat::from_rotation_z(v.to_radians());
        let mut joints = Vec::new();
        match self {
            Self::Rest => {}
            Self::Relaxed => joints.extend([("LeftArm", rz(40.)), ("RightArm", rz(-40.))]),
            Self::Reach => joints.extend([
                ("LeftArm", ry(-90.) * rz(-40.)),
                ("RightArm", ry(90.) * rz(40.)),
            ]),
            Self::Stride => joints.extend([
                ("LeftUpLeg", rx(40.)),
                ("RightUpLeg", rx(-30.)),
                ("LeftLeg", rx(-20.)),
                ("RightLeg", rx(-65.)),
                ("LeftArm", ry(20.) * rz(35.)),
                ("RightArm", ry(-25.) * rz(-35.)),
            ]),
            Self::Crouch => joints.extend([
                ("Spine", rx(-25.)),
                ("LeftUpLeg", rx(65.)),
                ("RightUpLeg", rx(65.)),
                ("LeftLeg", rx(-105.)),
                ("RightLeg", rx(-105.)),
                ("LeftFoot", rx(40.)),
                ("RightFoot", rx(40.)),
                ("LeftArm", ry(-55.)),
                ("RightArm", ry(55.)),
            ]),
            Self::Seated => joints.extend([
                ("LeftUpLeg", rx(90.)),
                ("RightUpLeg", rx(90.)),
                ("LeftLeg", rx(-90.)),
                ("RightLeg", rx(-90.)),
                ("LeftArm", ry(-90.) * rz(-15.)),
                ("RightArm", ry(90.) * rz(15.)),
            ]),
            Self::Twist => joints.extend([
                ("Spine", ry(20.)),
                ("Spine1", ry(20.)),
                ("LeftArm", ry(-60.) * rz(-20.)),
                ("RightArm", rz(-40.)),
            ]),
        }
        // Rotations are expressed in the common Y-up anatomical frame, then
        // converted through each rig's own authored parent basis.
        for (name, delta) in joints {
            let index = skeleton
                .bone_names()
                .iter()
                .position(|n| n == name)
                .ok_or_else(|| {
                    anyhow::anyhow!("pose {} requires missing bone {name}", self.name())
                })?;
            let models = skeleton.model_matrices(&pose)?;
            let (_, world_rotation, _) = models[index].to_scale_rotation_translation();
            let (scale, rotation, translation) = pose.local[index].to_scale_rotation_translation();
            let parent_rotation = world_rotation * rotation.inverse();
            pose.local[index] = Mat4::from_scale_rotation_translation(
                scale,
                parent_rotation.inverse() * delta * parent_rotation * rotation,
                translation,
            );
        }
        if matches!(self, Self::Crouch | Self::Seated) {
            let hips = skeleton
                .bone_names()
                .iter()
                .position(|n| n == "Hips")
                .ok_or_else(|| anyhow::anyhow!("pose requires Hips"))?;
            let height = skeleton.model_matrices(skeleton.rest_pose())?[hips]
                .w_axis
                .y;
            ensure!(height.is_finite(), "invalid hip height");
            pose.local[hips].w_axis.y -= height * if self == Self::Seated { 0.45 } else { 0.35 };
        }
        Ok(pose)
    }
}
