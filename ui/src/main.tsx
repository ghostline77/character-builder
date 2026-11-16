import { AppShell, Button, Input, Select, mountApp } from "@ghostline/ui-core";
import "./style.css";

function CharacterBuilder() {
  return (
    <AppShell
      product="Character builder"
      project={
        <div className="project-title">
          <strong id="project-name">Create your character</strong>
          <span id="saved-state">No project open</span>
        </div>
      }
      actions={
        <>
          <Button id="new">New</Button>
          <Button id="open">Open</Button>
          <Button id="import">Import</Button>
          <Button id="save" variant="primary">
            Save project
          </Button>
          <Button id="settings" aria-label="Local game settings">
            Settings
          </Button>
        </>
      }
    >
      <main>
        <aside className="library">
          <div className="section-heading">
            <h1>Catalogue</h1>
          </div>
          <label className="search">
            <span className="sr-only">Search catalogue</span>
            <Input
              id="search"
              type="search"
              placeholder="Hair, jackets, chrome…"
            />
          </label>
          <label className="category-label">
            Category
            <Select id="category" defaultValue="hair_color">
              <option value="hair_color">Hair</option>
            </Select>
          </label>
          <div className="library-meta">
            <span id="result-count">Loading catalogue…</span>
            <Button id="previous" size="sm" aria-label="Previous page">
              ←
            </Button>
            <Button id="next" size="sm" aria-label="Next page">
              →
            </Button>
          </div>
          <div id="cards" className="cards" aria-live="polite" />
        </aside>
        <section className="stage">
          <div className="stage-top">
            <div id="appearances" className="tabs" />
            <Button id="add-appearance" title="Duplicate appearance">
              + Appearance
            </Button>
            <Button id="remove-appearance" title="Remove appearance">
              −
            </Button>
          </div>
          <div id="viewport" className="viewport">
            <div className="empty" id="empty">
              <span className="orbit-mark">✳</span>
              <h2>No character open</h2>
              <p>Create a character or open a saved project.</p>
              <Button id="empty-new" variant="primary">
                Create a character
              </Button>
            </div>
            <img id="preview" alt="Assembled character preview" hidden />
            <span id="preview-state" className="preview-state" hidden />
          </div>
          <div className="camera">
            <Button id="camera-front">Front</Button>
            <Button id="camera-hero">¾ view</Button>
            <Button id="camera-back">Back</Button>
            <label>
              Orbit
              <Input
                id="yaw"
                type="range"
                min="-3.14"
                max="3.14"
                step="0.02"
                defaultValue="-0.55"
              />
            </label>
            <label>
              Zoom
              <Input
                id="zoom"
                type="range"
                min="0.5"
                max="4"
                step="0.05"
                defaultValue="1"
              />
            </label>
          </div>
          <div className="stage-bottom">
            <div>
              <span id="job-state" role="status" aria-live="polite">
                Ready
              </span>
              <small id="source-state" hidden />
            </div>
            <Button id="cancel-job" hidden>
              Cancel
            </Button>
            <Button id="prepare" variant="primary">
              Update preview
            </Button>
          </div>
        </section>
        <aside className="inspector">
          <div className="inspector-heading">
            <h2>Character</h2>
            <div className="history">
              <Button id="undo" title="Undo">
                ↶ Undo
              </Button>
              <Button id="redo" title="Redo">
                ↷ Redo
              </Button>
            </div>
          </div>
          <label>
            Name
            <Input id="name" placeholder="Character name" />
          </label>
          <div className="template-info" id="template-info">
            No template selected
          </div>
          <label>
            Appearance name
            <Input id="appearance-name" />
          </label>
          <details open>
            <summary>Face shape</summary>
            <div id="shapes" className="shapes" />
          </details>
          <details open>
            <summary>Selected appearance</summary>
            <div id="selected" className="selected" />
          </details>
          <details open>
            <summary>Weapons / loadout</summary>
            <div id="loadout" className="selected" />
          </details>
          <details>
            <summary>NPC behaviour</summary>
            <label>
              Find behaviour presets
              <Input
                id="ai-preset-search"
                type="search"
                placeholder="Netrunner, melee, police, faction…"
              />
            </label>
            <label>
              Behaviour preset
              <Select id="ai-profile" defaultValue="quest">
                <option value="quest">Quest NPC</option>
                <option value="combat">Combat NPC</option>
                <option value="cyberpsycho_netrunner">
                  Cyberpsycho · netrunner
                </option>
              </Select>
            </label>
            <p id="ai-preset-info" hidden />
            <details className="advanced-settings">
              <summary>Advanced behaviour</summary>
              {[
                ["ai-action-map", "Action map"],
                ["ai-archetype", "Archetype"],
                ["ai-attitude", "Attitude group"],
                ["ai-reaction", "Reaction preset"],
                ["ai-rarity", "Rarity"],
              ].map(([id, label]) => (
                <label key={id}>
                  {label}
                  <Select id={id} />
                </label>
              ))}
            </details>
            <label>
              Power level
              <Input
                id="ai-level"
                type="number"
                min="1"
                max="100"
                placeholder="Inherit game scaling"
              />
            </label>
            <label>
              Health multiplier
              <Input
                id="ai-health"
                type="number"
                min="0.01"
                step="0.1"
                placeholder="Inherit preset"
              />
            </label>
          </details>
          <details>
            <summary>Export identity</summary>
            {[
              ["record", "Record"],
              ["namespace", "Depot namespace"],
              ["voice", "Voice tag"],
              ["affiliation", "Affiliation"],
            ].map(([id, label]) => (
              <label key={id}>
                {label}
                <Input id={id} />
              </label>
            ))}
          </details>
          <div className="export-actions">
            <Button id="validate">Validate</Button>
            <Button id="download">Project JSON</Button>
            <Button id="export">Export character</Button>
          </div>
          <div id="messages" role="status" aria-live="polite" />
        </aside>
      </main>
      <dialog id="new-dialog">
        <form id="new-form">
          <h2>New character</h2>
          <label>
            Project ID
            <Input
              id="new-id"
              required
              pattern="[a-zA-Z0-9_-]{1,80}"
              placeholder="my_character"
            />
          </label>
          <label>
            Body template
            <Select id="new-template" />
          </label>
          <div className="dialog-actions">
            <Button data-close>Cancel</Button>
            <Button type="submit" variant="primary">
              Create character
            </Button>
          </div>
        </form>
      </dialog>
      <dialog id="open-dialog">
        <h2>Your characters</h2>
        <div id="project-list" />
        <div className="dialog-actions">
          <Button data-close>Close</Button>
        </div>
      </dialog>
      <dialog id="settings-dialog">
        <form id="settings-form">
          <h2>Game resources</h2>
          <p>Required for character previews and exports.</p>
          <label>
            Cyberpunk 2077 directory
            <Input id="game-path" placeholder="K:/Cyberpunk 2077" />
          </label>
          <label>
            Reflection override (optional)
            <Input id="schema-path" placeholder="Automatic game metadata" />
          </label>
          <div className="dialog-actions">
            <Button data-close>Cancel</Button>
            <Button type="submit" variant="primary">
              Use resources
            </Button>
          </div>
        </form>
      </dialog>
      <Input id="import-file" type="file" accept=".json" hidden />
    </AppShell>
  );
}
mountApp(<CharacterBuilder />, async () =>
  (await import("../app.js")).startController(),
);
