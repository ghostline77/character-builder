# Native actor shells

The JSON files under [data/templates](../data/templates) supply native entity and
appearance structure for supported character frames. They are builder inputs,
not complete exported characters.

Each template has an `.ent-shell.json` and `.app-shell.json` pair. Player
frames use the `npv-` prefix; average, big and fat NPC frames use the `npc-`
prefix with their sex and body family. Template selection and file lookup live in
[src/templates.rs](../src/templates.rs).

The character project chooses its frame and supplies appearance selections,
equipment, face edits and loadout. The builder combines these with the selected
shell and writes project-owned native resources. Changing an exported binary does
not change the template or the saved character project.

When adding a frame, update its template definition and shell pair together.
Check component identities, appearance references, rig bindings, native resource
dependencies and generated depot paths. Review both head and full-body output,
then test animation and clothing in game. A matching filename alone does not
establish compatibility with another body's rig.
