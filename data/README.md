# Character templates

`templates` contains the sixteen reviewed entity/appearance JSON shells for the
eight frame profiles, copied from the Ghostline authoring template collection.
Source: `quests/templates/characters/templates` in the Ghostline repository.
These are resource authoring inputs, not game binaries or generated previews.

Player male/female, average NPC, big NPC and fat NPC templates remain distinct.
Frame names describe the template profile; they do not establish clothing fit.
`frames` contains the reviewed native donor mappings, body families and provenance.
Donor meshes and textures are read from the installed game. Preview and export do
not depend on tutorial NPC meshes or their embedded material paths.

The entity shells retain their compiled runtime component graph. Unreflected
editor component duplicates, editor headers, stale cached dependencies, missing
Kerenzikov FX and an unavailable spy effect were removed. Obsolete EP1 death clips
use existing base-game clips; female-big shells omit unavailable male-big knife
animation clips. Player shells use their native deformation rigs for HQ skinning.
Valid wounds, dismemberment and actor support data remain part of the runtime shell.

Export rebuilds dependency declarations from the authored graph and checks the
written resources, including mesh-local material instances, for missing references
and tutorial paths. Missing optional animation setup clips are omitted explicitly
and recorded in export diagnostics; required dependencies still fail export.

Catalogue choices and thumbnails are read from a separately published `items`
package. They are not duplicated into this directory.
