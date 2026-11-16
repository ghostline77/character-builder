# Character projects, previews and export

## Character projects

Projects retain exact native creator control/choice identities, equipment selections,
face shape values and multiple named appearances. Their catalogue identity excludes
media generation progress. Saves use atomic replacement and revision checks; the UI
supports undo, redo, JSON import/export and recovery of unsaved edits.

Opening or importing an older project checks its selections against the current
catalogue instead of requiring an identical catalogue release. Missing female
lower-leg/foot baselines are restored with the linked body colour, and former
weapon appearance selections move into the weapon loadout. The UI reports these
adjustments and marks the project as unsaved; save normally to keep them. Missing
or invalid selections remain reported for correction.

Female player bodies require separate arms, torso, lower-leg/foot and head selections.
Male player bodies include their lower legs and feet in the torso selection. Defaults
include eyes, eyebrows, eyelashes and teeth. Native linked colour controls update
the selected body components together. Explicit equipment appearance conditions
filter the picker and are validated when loading authored projects.

The clothing picker also consumes individual native garments from the catalogue's
clothing selections, independently of inventory records. Garments share the normal
clothing categories, group their material appearances under one style, and use the
authored body-family token to select compatible builder templates. First-person
companions and complete vanilla NPC outfits are omitted from the picker. Garment
meshes retain their owning native component and deformation rig when available;
the same assembly is used for previews and exports. Child and special-purpose
body families require a corresponding builder template before they can be worn.

## Native frames, skin and previews

Previews use the shared native renderer on the GPU. Native geometry, component masks,
materials and morph targets are retained; camera and shape changes reuse prepared
scenes. Preview results carry their project revision and appearance. Results from an
older edit cannot replace the current preview.

Player, average NPC, big NPC and fat NPC frames use their native bodies and rigs.
NPC heads are selected from the reviewed frame library. Creator face shapes transfer
to donor geometry with a protected neck collar; export bakes those changes into all
mesh LODs. Face and Full body controls provide separate preview framing.

The NPC Skin colour control changes the head and body together. Match donor head
uses the donor's authored tone. Explicit colours select native palette appearances;
heads without a matching appearance receive a native face-palette material override.
The same material edits are used in previews and exported meshes.

The picker groups customization styles before their colours or variants. Internal
inventory categories and attachment parts are omitted. Clothing includes inventory
items and individual native garments published by Items. Coverage depends on the
catalogue and on the available body-family templates.
Player clothing does not automatically acquire another body's proportions. Native
component bindings, chunk masks and visual hiding tags are evaluated, but runtime
garment deformation and arbitrary clothing fit remain unverified.

Skin shading currently omits subsurface scattering and several native detail layers.
Rendering is an asset preview, not an in-game appearance certification.

## Export and combat loadouts

`render-animations PROJECT --catalogue CATALOGUE --game GAME
--clips MANIFEST --out DIRECTORY` renders short native body clips through the
character preview assembly. The manifest is an array of objects containing
`label`, `animset` and `animation`. The command requires `refit_clothing: false`
and disables fitting during preview assembly. It writes timestamped PNG sequences
and a report at 12 FPS, up to five seconds per clip. This is a diagnostic path;
missing rig bindings and animation deformation still require visual review.

Weapons belong to Weapons / loadout and are omitted from appearance geometry.
Loadout choices export as TweakXL NPCEquipmentGroup and NPCEquipmentItem records,
with native weapon slots and equip/unequip conditions. Quest and combat NPC bases
are separate. The Cyberpsycho netrunner preset follows the Goth Baddie example,
including its action map, archetype, abilities, effectors and native stat groups.
Action map, archetype and attitude can be overridden; fixed level and health
multiplier records are optional. A loadout alone does not configure every AI tactic.

Export produces an archive, TweakXL YAML, portable character project and report.
Install the archive in `archive/pc/mod` and YAML in `r6/tweaks`; TweakXL is required.
Native serialization is decoded back to verify component identities, visibility,
mesh geometry and local material parameters. Dependency checks reject missing
required resources and tutorial-owned paths. Reports list omitted unavailable
optional animation clips. Installed REDmod reflection metadata supplements the
automatically loaded reflection metadata; no REDmod executable is run.

In-game spawning, combat behavior, deformation and fit still require playtesting.

For a reproducible GPU review across every frame, alternate heads/tones and a
representative outfit, run:

```powershell
target/release/ghostline-character-builder.exe review --game 'path/to/Cyberpunk 2077' --out work/review --export
```

Completed cases are resumable. Inspect `review.json` and the body/head images;
successful resource serialization alone does not establish clothing fit.

`work/` holds generated previews, job receipts and local projects. `data/templates/`
contains reviewed native actor shells; see the [native actor shell guide](templates.md).

## Clothing layer fitting

Clothing fitting is experimental and off by default. Enable **Refit clothing**
in the character inspector to apply it to both previews and exports. The setting
is saved as `refit_clothing` in the project; saves without it load with fitting
disabled. With it off, generated garment fitting and coverage masks are skipped.
`review-fits` explicitly enables fitting for its review projects unless
`--unfitted` is supplied.

The builder evaluates selected clothing together using the native component-prefix
scores and garment size tags. Shared bind-space garments use authored support-morph
paths when available, with skeleton guides for meshes without a custom support
morph. Body surfaces bound compression. Body and clothing contacts use the
renderer’s calibrated assembly transforms; sharing a root component does not
imply a shared mesh bind space. Corrections map back through each vertex’s
inverse skin transform. Only the highest-detail anatomical body surfaces
constrain the fit. Layers are processed from outside to inside. Geometry fits are
cached independently of colour and material appearance.

`ghostline-red` decodes support proxies, offsets, flags and cap lists. The sibling
`refitter` library owns surface constraints and accelerated collision queries.
The builder bakes the resulting positions across native LODs, updates shading
vectors and retains UVs, skin weights, support data and native bindings.
Preview and export consume the same edited mesh documents. Fit measurements,
cache reuse and unsupported binding contexts are recorded in source diagnostics.

Fitting also evaluates relaxed arms, forward reach, stride, crouch, seated and
torso-twist stress poses on the character's native locomotion rig. Linked native
deformation and garment rigs follow that motion. Vertex, edge and face contacts
drive local boundary corrections across native LODs while retaining the authored
outer garment shape. Attached stitch/decal meshes follow their cloth receivers'
deformation and visibility. A coverage pass masks body and inner-clothing triangles under opaque
outer clothing only when matching coverage remains in every checked pose.
Exposed openings, opposing surfaces and transparent/masked materials do not
establish coverage. The pass retains vertex streams and native chunk slots;
masked faces become degenerate triangles in preview and exported native meshes.
At concave joints, an additional coverage pass tests the inline native garment
morph direction and the cloth normal. It masks only connected regions containing
a measured triangle intersection, with coverage at vertices, edge midpoints and
face centres in every checked pose. Native protected caps remain open. This pass
changes indices only; it does not inflate or shrink an otherwise working outfit.

Combined torso/leg garments retain their complete native geometry when worn
alone. When separate trousers are selected, the builder identifies the combined
garment's leg surfaces through skin bindings, connected topology and enclosure of
both thighs, and masks those surfaces across its LODs. A long shirt or coat hem
alone does not qualify as a combined garment. The separately selected trousers
are retained unchanged by this part-selection step.

The fit cache includes geometry, topology, skin weights, rigs, attachment
frames and body visibility; changing an appearance does not require another fit.

Run a repeatable clothing review across all eight supported body templates:

```powershell
cargo run --release -- review-fits --game "path/to/Cyberpunk 2077" --out work/fit-review
```

`--project path/to/character.json` adds an authored character; a directory loads
its `.character.json` projects. Use
`--samples-per-rig 0` to review only supplied projects, or `--unfitted` for an
original-geometry comparison. The command writes front/side pose renders,
portable projects, collision candidates and an incremental evidence report.
Collision candidates require visual assessment: open garment boundaries,
intentional holes and hidden surfaces can produce geometric contacts.

These are deterministic skeletal stress checks, not cloth physics or exhaustive
game-animation certification. Arbitrary cross-skeleton weight transfer remains
a separate capability.

## Mechanical cyberware fitting

`review-cyberware` can explicitly fit the NPC mantis-blade assembly to `male_big`.
It rebases the mechanical rig onto the target's named anatomical anchors, then
updates mesh positions and inverse bind matrices together. Mechanical child
transforms, skin weights, UVs, materials and native LODs are retained. This path
is independent of the character's experimental clothing-refit setting.

```powershell
cargo run --release -- review-cyberware --game "path/to/Cyberpunk 2077" --template male_big --entity 'base\characters\cyberware\npc\psycho_mantisblade.ent' --fit --out work/mantis-big
```

The output contains pose previews, native retraction-animation samples, a fit
report, editable resource JSON and native mesh/rig/entity files under
`native/mod/ghostline_refits/male_big`. The native files are decoded again to
verify geometry, skin bindings and rig transforms. They are candidate weapon
assets; NPC loadout/behaviour integration and an in-game deployment check remain
separate. The command does not install or replace game resources.

Use `--project character.json` to review the fit on an authored outfit and
`--depot 'mod\my_quest\cyberware'` to publish assets in the consuming quest's
namespace. `--overlay candidate.archive` can reopen the published native entity
for a comparison without installing it.
