//! Native components for authored mesh assets outside inventory records.
use crate::project::NativePart;
use anyhow::Result;
use ghostline_red::assets::{Assembly, ResourceKey};
use ghostline_renderer::GameSource;
use serde_json::{Value, json};

fn name(value: &str) -> Value {
    json!({"$type":"CName","$storage":"string","$value":value})
}
fn resource(path: &str) -> Value {
    json!({"DepotPath":{"$type":"ResourcePath","$storage":"string","$value":path},"Flags":"Default"})
}

pub fn assemble(part: &NativePart, source: &GameSource) -> Result<Assembly> {
    let animated = format!("{}_rig", part.id);
    let binding = if part.rig.is_some() {
        animated.as_str()
    } else {
        "root"
    };
    let mut handle = 0;
    let mut wrap = |data: Value| {
        let id = handle;
        handle += 1;
        json!({"HandleId":id.to_string(),"Data":data})
    };
    let parent = wrap(
        json!({"$type":"entHardTransformBinding","bindName":name(binding),"slotName":name("None")}),
    );
    let skin = wrap(json!({"$type":"entSkinningBinding","bindName":name(binding)}));
    let mesh = wrap(
        json!({"$type":"entSkinnedMeshComponent","name":name(&part.id),"mesh":resource(&part.mesh),"meshAppearance":name(&part.appearance),"isEnabled":1,"chunkMask":u64::MAX.to_string(),"parentTransform":parent,"skinning":skin}),
    );
    let mut components = vec![mesh];
    if let (Some(rig), Some(graph)) = (&part.rig, &part.graph) {
        let parent = wrap(
            json!({"$type":"entHardTransformBinding","bindName":name("root"),"slotName":name("None")}),
        );
        let control = wrap(json!({"$type":"entAnimationControlBinding","bindName":name("root")}));
        components.push(wrap(json!({"$type":"entAnimatedComponent","name":name(&animated),"rig":resource(rig),"graph":resource(graph),"isEnabled":1,"parentTransform":parent,"controlBinding":control})));
    }
    if let Some(shadow) = &part.shadow {
        let parent = wrap(
            json!({"$type":"entHardTransformBinding","bindName":name("root"),"slotName":name("None")}),
        );
        let skin = wrap(json!({"$type":"entSkinningBinding","bindName":name("root")}));
        components.push(wrap(json!({"$type":"entSkinnedMeshComponent","name":name(&format!("{}_shadow",part.id)),"mesh":resource(shadow),"meshAppearance":name("default"),"isEnabled":1,"chunkMask":u64::MAX.to_string(),"parentTransform":parent,"skinning":skin})));
    }
    let document =
        json!({"Data":{"RootChunk":{"$type":"entEntityTemplate","components":components}}});
    Ok(source
        .resources()
        .assembly_document(&ResourceKey::parse(&part.mesh), &document, None)?)
}
