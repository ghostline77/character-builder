export interface Appearance {
  id: string;
  name: string;
  choices: Record<string, string>;
  equipment: Record<string, string>;
  shapes: Record<string, number>;
  native_outfit?: string | null;
  native_head?: string | null;
  skin_tone?: string | null;
  [key: string]: unknown;
}
export interface Project {
  id: string;
  name: string;
  revision: number;
  template: string;
  namespace: string;
  active_appearance: string;
  appearances: Appearance[];
  identity: {
    record: string;
    voice_tag: string;
    affiliation: string;
    [key: string]: unknown;
  };
  loadout: string[];
  ai: Record<string, string | number | null>;
  refit_clothing?: boolean;
  [key: string]: unknown;
}
export interface Entry {
  id: string;
  label: string;
  category: string;
  kind: string;
  resolution: string;
  appearance?: string;
  variants?: Entry[];
  media?: { path: string; view: string }[];
  style?: string;
  garment_style?: string;
  variant_label?: string;
  record?: string;
  role?: string;
  choice?: string;
  control?: string;
  control_name?: string;
  templates?: string[];
  template?: string;
  native_outfit?: string;
  body_family?: string;
}
export interface Template {
  id: string;
  label: string;
  frame: string;
  shape_max: number;
  head_mode?: string;
}
export interface Bootstrap {
  templates: Template[];
  categories: string[];
  settings: { game: string | null; schema: string | null };
  native_heads?: Record<
    string,
    { id: string; label: string; default?: boolean }[]
  >;
  behaviour: {
    presets: {
      id: string;
      source_record: string;
      values: Record<string, string>;
    }[];
    options: Record<string, string[]>;
  };
  catalogue: { sources: { game_version?: string } };
}
export interface Prepared {
  project: Project;
  adjustments: string[];
  validation: { ok: boolean; errors: string[]; warnings: string[] };
}
export interface Metadata {
  choice_selections: Record<string, string>;
  choice_entries: Record<string, Entry>;
}
export interface Job {
  id: string | number;
  state: string;
  kind: string;
  project: string;
  result?: {
    scene?: string;
    adapter?: string;
    warnings?: string[];
    error?: string;
    head_bounds?: { center: [number, number, number]; radius: number };
  };
}
export interface Camera {
  yaw: number;
  pitch: number;
  zoom: number;
  size: number;
  focus_head: boolean;
  browser_scene: boolean;
}
export async function api<T>(
  path: string,
  data?: unknown,
  signal?: AbortSignal,
): Promise<T> {
  const response = await fetch(path, {
    ...(data === undefined
      ? {}
      : {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify(data),
        }),
    ...(signal ? { signal } : {}),
  });
  const value = (await response.json()) as T & { error?: string };
  if (!response.ok) throw Error(value.error || response.statusText);
  return value;
}
export const appearance = (project: Project): Appearance =>
  project.appearances.find((a) => a.id === project.active_appearance) ||
  project.appearances[0]!;
export const displayLabel = (entry: Entry): string => {
  const value = (entry.label || entry.appearance || entry.id)
    .replace(/^UI-CharacterCreation-/, "")
    .replace(/^\d+[_-]/, "")
    .replaceAll("_", " ");
  return (
    (entry.style ? "Style " + entry.style + " · " : "") +
    value.charAt(0).toUpperCase() +
    value.slice(1)
  );
};
export const categoryLabel = (value: string): string =>
  value
    .replace(/^(Clo_|Wea_)/, "")
    .replace(/([a-z])([A-Z])/g, "$1 $2")
    .replaceAll("_", " ");
export function selectedIds(project: Project, metadata: Metadata): Set<string> {
  const a = appearance(project);
  return new Set([
    ...(a.native_outfit
      ? [`npc-outfit:${project.template}:${a.native_outfit}`]
      : []),
    ...project.loadout,
    ...Object.values(a.equipment),
    ...Object.values(a.choices).flatMap((id) =>
      metadata.choice_selections[id] ? [metadata.choice_selections[id]!] : [],
    ),
  ]);
}
export function localSelection(
  project: Project,
  entry: Entry,
  metadata: Metadata,
): void {
  const a = appearance(project);
  if (entry.kind === "npc_clothing") {
    if (entry.template !== project.template)
      throw Error("Choose the matching NPC body template: " + entry.template);
    a.native_outfit = entry.native_outfit || null;
    a.equipment = {};
  } else if (entry.kind === "loadout") {
    project.loadout = project.loadout.filter(
      (id) => metadata.choice_entries[id]?.role !== entry.role,
    );
    project.loadout.push(entry.id);
    if (!project.ai.profile || project.ai.profile === "quest")
      project.ai.profile = "combat";
  } else {
    if (
      entry.kind === "equipment" &&
      !entry.templates &&
      /_(big|fat)$/.test(project.template)
    )
      throw Error(
        "This player garment does not fit the selected big/fat body. Native clothing for that rig is required.",
      );
    a.native_outfit = null;
    a.equipment[entry.category || "equipment"] = entry.id;
  }
}
