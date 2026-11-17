# Ghostline Character Builder

Create Cyberpunk 2077 characters, choose body and face appearances, equip clothing
and define NPC weapon loadouts in a local browser editor.

## Start the editor

Install Rust 1.92+, Node.js 20+ and GitHub access to the Ghostline dependencies. Run from this repository. The executable embeds its UI, native templates, frame recipes and behaviour catalogue. Supply the item catalogue separately.

```powershell
cargo build --release --locked
./target/release/ghostline-character-builder.exe serve --catalogue path/to/items-catalogue
```

Open <http://127.0.0.1:8787/>. Browsing and project editing work without the game.
Set the installed game directory in **Settings** for assembled previews and native
exports. Reflection metadata loads automatically.

## Create a character

1. Create a project and choose its body template and head.
2. Choose appearance styles, then colours. Skin controls update the head and body together.
3. Add compatible clothing. Put weapons in **Weapons / loadout**; they do not attach to the preview.
4. Inspect **Face** and **Full body** views. Drag to orbit, right-drag to pan and scroll to zoom.
5. Save the project, then export when ready.

Project JSON retains native selections and named appearances. An export contains
native resources, TweakXL records, the portable project and a report. Follow the
[export instructions](docs/authoring.md#export-and-combat-loadouts) for installation.
Test spawning, animation and combat in game.

- [Projects, body compatibility and linked skin controls](docs/authoring.md#character-projects)
- [Preview behaviour and limitations](docs/authoring.md#native-frames-skin-and-previews)
- [Loadouts and export](docs/authoring.md#export-and-combat-loadouts)
- [Clothing fitting](docs/authoring.md#clothing-layer-fitting)
- [Mechanical cyberware fitting](docs/authoring.md#mechanical-cyberware-fitting)
- [Native actor shells](docs/templates.md)

Native decoding and mesh baking belong to Ghostline Red; rendering belongs to
Renderer. The builder owns character selection, project editing and export rules.
