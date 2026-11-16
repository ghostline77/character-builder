import { useEffect, useMemo, useRef, useState } from "react";
import type { ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { AppShell, Button, Input, Select } from "@ghostline/ui-core";
import { Viewport, nativeMaterials } from "@ghostline/ui-core/viewport";
import type {
  ViewportHandle,
  ViewportSource,
} from "@ghostline/ui-core/viewport";
import {
  api,
  appearance,
  categoryLabel,
  displayLabel,
  localSelection,
  selectedIds,
} from "./model";
import type {
  Bootstrap,
  Camera,
  Entry,
  Job,
  Metadata,
  Prepared,
  Project,
} from "./model";
import "./style.css";
interface EditorState {
  project: Project | null;
  saved: number | null;
  dirty: boolean;
  generation: number;
  undo: Project[];
  redo: Project[];
}
const initial: EditorState = {
  project: null,
  saved: null,
  dirty: false,
  generation: 0,
  undo: [],
  redo: [],
};
const aiFields = [
  ["action_map", "actionMap", "Action map"],
  ["archetype_data", "archetypeData", "Archetype"],
  ["attitude_group", "baseAttitudeGroup", "Attitude group"],
  ["reaction_preset", "reactionPreset", "Reaction preset"],
  ["rarity", "rarity", "Rarity"],
] as const;
function Modal({
  title,
  children,
  close,
}: {
  title: string;
  children: ReactNode;
  close: () => void;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    ref.current?.showModal();
  }, []);
  return (
    <dialog
      ref={ref}
      onCancel={close}
      className="m-auto max-h-[90dvh] w-[min(620px,95vw)] overflow-auto rounded-xl border border-line bg-panel p-6 text-foreground backdrop:bg-black/70"
    >
      <div className="mb-5 flex justify-between gap-4">
        <h2 className="text-xl font-semibold">{title}</h2>
        <Button onClick={close}>Close</Button>
      </div>
      {children}
    </dialog>
  );
}
function CharacterBuilder() {
  const [bootstrap, setBootstrap] = useState<Bootstrap | null>(null),
    [editor, setEditor] = useState<EditorState>(initial),
    [metadata, setMetadata] = useState<Metadata>({
      choice_entries: {},
      choice_selections: {},
    }),
    [message, setMessage] = useState(""),
    [bad, setBad] = useState(false),
    [modal, setModal] = useState<"new" | "open" | "settings" | null>(null),
    [projects, setProjects] = useState<
      {
        id: string;
        name: string;
        file: string;
        template: string;
        revision: number;
        error?: string;
      }[]
    >([]);
  const [query, setQuery] = useState(""),
    [category, setCategory] = useState("hair_color"),
    [offset, setOffset] = useState(0),
    [results, setResults] = useState<{ entries: Entry[]; total: number }>({
      entries: [],
      total: 0,
    }),
    [searching, setSearching] = useState(false),
    [presetQuery, setPresetQuery] = useState("");
  const [newId, setNewId] = useState(""),
    [newTemplate, setNewTemplate] = useState(""),
    [settings, setSettings] = useState({ game: "", schema: "" }),
    [job, setJob] = useState<{ id: string; generation: number } | null>(null),
    [jobState, setJobState] = useState("Ready"),
    [source, setSource] = useState<ViewportSource | undefined>(),
    [previewGeneration, setPreviewGeneration] = useState(-1),
    [downloads, setDownloads] = useState<{ href: string; label: string }[]>([]),
    [draft, setDraft] = useState(false),
    [saving, setSaving] = useState(false);
  const [camera, setCamera] = useState<Camera>({
    yaw: -0.55,
    pitch: 0.12,
    zoom: 1,
    size: 768,
    focus_head: false,
    browser_scene: true,
  });
  const viewport = useRef<ViewportHandle>(null),
    file = useRef<HTMLInputElement>(null),
    latest = useRef(editor),
    headBounds = useRef<
      { center: [number, number, number]; radius: number } | undefined
    >(undefined);
  latest.current = editor;
  const project = editor.project,
    a = project ? appearance(project) : undefined,
    template = bootstrap?.templates.find((t) => t.id === project?.template);
  const notify = (value: string, error = false): void => {
    setMessage(value);
    setBad(error);
  };
  const run = (action: () => unknown | Promise<unknown>): void => {
    Promise.resolve()
      .then(action)
      .catch((e) => notify(e instanceof Error ? e.message : String(e), true));
  };
  const load = (
    value: Project,
    saved: number | null = null,
    dirty = false,
  ): void => {
    const next = structuredClone(value);
    next.loadout ||= [];
    next.ai ||= { profile: "quest" };
    setEditor((current) => ({
      project: next,
      saved,
      dirty,
      generation: current.generation + 1,
      undo: [],
      redo: [],
    }));
    setSource(undefined);
    setPreviewGeneration(-1);
    setOffset(0);
    headBounds.current = undefined;
  };
  const prepared = (
    result: Prepared,
    saved: number | null = null,
    imported = false,
  ): void => {
    load(result.project, saved, imported || result.adjustments.length > 0);
    notify(
      [
        ...result.adjustments,
        ...result.validation.errors,
        ...result.validation.warnings,
        ...(imported ? ["Save to keep this character in your library."] : []),
      ].join("\n") || "Character loaded.",
      !result.validation.ok,
    );
  };
  const edit = (change: (project: Project) => void): void => {
    const current = latest.current;
    if (!current.project) return;
    const next = structuredClone(current.project);
    change(next);
    const updated = {
      ...current,
      project: next,
      dirty: true,
      generation: current.generation + 1,
      undo: [...current.undo, current.project].slice(-80),
      redo: [],
    };
    latest.current = updated;
    setEditor(updated);
  };
  const discard = (): boolean =>
    !latest.current.dirty ||
    window.confirm(
      "Leave this character with unsaved changes? Download or save it first to keep them.",
    );
  useEffect(() => {
    let active = true;
    api<Bootstrap>("/api/bootstrap")
      .then((value) => {
        if (active) {
          setBootstrap(value);
          setNewTemplate(value.templates[0]?.id || "");
          setSettings({
            game: value.settings.game || "",
            schema: value.settings.schema || "",
          });
          setDraft(!!localStorage.getItem("ghostline-character-draft"));
        }
      })
      .catch((e) => {
        if (active) notify(String(e), true);
      });
    return () => {
      active = false;
    };
  }, []);
  useEffect(() => {
    if (editor.dirty && editor.project) {
      try {
        localStorage.setItem(
          "ghostline-character-draft",
          JSON.stringify(editor.project),
        );
      } catch {
        notify(
          "Browser storage is unavailable. Save or download this character to keep your changes.",
          true,
        );
      }
    }
    const unload = (e: BeforeUnloadEvent) => {
      if (latest.current.dirty) {
        e.preventDefault();
        e.returnValue = "";
      }
    };
    addEventListener("beforeunload", unload);
    return () => removeEventListener("beforeunload", unload);
  }, [editor.generation]);
  useEffect(() => {
    if (!bootstrap) return;
    const abort = new AbortController();
    const timer = setTimeout(() => {
      setSearching(true);
      const params = new URLSearchParams({
        q: query,
        category,
        frame: template?.frame || "",
        template: project?.template || "",
        offset: String(offset),
      });
      api<{ entries: Entry[]; total: number }>(
        "/api/catalogue?" + params,
        undefined,
        abort.signal,
      )
        .then((value) => {
          if (!abort.signal.aborted) setResults(value);
        })
        .catch((e) => {
          if (!abort.signal.aborted) notify(String(e), true);
        })
        .finally(() => {
          if (!abort.signal.aborted) setSearching(false);
        });
    }, 200);
    return () => {
      clearTimeout(timer);
      abort.abort();
    };
  }, [bootstrap, query, category, offset, project?.template]);
  useEffect(() => {
    if (!project) return;
    const abort = new AbortController();
    const timer = setTimeout(() => {
      api<Metadata>("/api/project-selections", { project }, abort.signal)
        .then((value) => {
          if (!abort.signal.aborted)
            setMetadata((previous) => ({
              choice_entries: {
                ...previous.choice_entries,
                ...value.choice_entries,
              },
              choice_selections: {
                ...previous.choice_selections,
                ...value.choice_selections,
              },
            }));
        })
        .catch((e) => {
          if (!abort.signal.aborted) notify(String(e), true);
        });
    }, 100);
    return () => {
      clearTimeout(timer);
      abort.abort();
    };
  }, [editor.generation]);
  useEffect(() => {
    if (!job) return;
    let active = true,
      timer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      try {
        const result = await api<Job>("/api/jobs/" + job.id);
        if (!active) return;
        setJobState(
          result.state === "complete"
            ? result.kind === "export"
              ? "Export ready"
              : "Ready"
            : result.state,
        );
        if (["complete", "failed", "cancelled"].includes(result.state)) {
          setJob(null);
          if (result.state === "complete" && result.kind === "preview") {
            if (job.generation !== latest.current.generation) {
              notify("Preview out of date. Update it to show your changes.");
              return;
            }
            headBounds.current = result.result?.head_bounds;
            if (result.result?.scene) {
              setSource({
                kind: "gltf",
                url: `/output/${job.id}/${result.result.scene}`,
              });
              setPreviewGeneration(job.generation);
            }
            notify((result.result?.warnings || []).join("\n"));
          } else if (result.state === "complete") {
            notify(
              "Export complete. Install the archive in archive/pc/mod and the TweakXL YAML in r6/tweaks. TweakXL is required.",
            );
            setDownloads(
              [
                [result.project + ".archive", "Character archive"],
                [result.project + ".yaml", "TweakXL · loadout and behaviour"],
                ["export-report.json", "Export report"],
              ].map(([name, label]) => ({
                href: `/output/${job.id}/${encodeURIComponent(name!)}`,
                label: label!,
              })),
            );
          } else
            notify(
              result.result?.error || result.state,
              result.state === "failed",
            );
          return;
        }
        timer = setTimeout(() => void poll(), 750);
      } catch (e) {
        if (active) {
          setJob(null);
          notify(String(e), true);
        }
      }
    };
    void poll();
    return () => {
      active = false;
      clearTimeout(timer);
    };
  }, [job?.id]);
  const startJob = async (exporting = false): Promise<void> => {
    if (!project) return;
    const value = await api<{ id: string | number }>("/api/jobs", {
      project,
      export: exporting,
      camera,
    });
    setJob({ id: String(value.id), generation: editor.generation });
    setJobState(exporting ? "Export queued…" : "Preview queued…");
    setDownloads([]);
  };
  const select = async (entry: Entry): Promise<void> => {
    if (!project) {
      setModal("new");
      return;
    }
    if (entry.choice) {
      const generation = editor.generation,
        updated = await api<Project>("/api/choices/select", {
          project,
          choice: entry.choice,
        });
      if (generation !== latest.current.generation) return;
      edit((value) => Object.assign(value, updated));
      setMetadata((current) => ({
        choice_entries: { ...current.choice_entries, [entry.choice!]: entry },
        choice_selections: {
          ...current.choice_selections,
          [entry.choice!]: entry.id,
        },
      }));
    } else {
      edit((value) => localSelection(value, entry, metadata));
      setMetadata((current) => ({
        ...current,
        choice_entries: { ...current.choice_entries, [entry.id]: entry },
      }));
    }
    notify(displayLabel(entry) + " selected.");
  };
  const save = async (): Promise<void> => {
    if (!project || saving) return;
    setSaving(true);
    const generation = editor.generation;
    try {
      const saved = await api<{ revision: number }>("/api/projects", {
        project,
        expected_revision: editor.saved,
      });
      setEditor((current) =>
        current.project?.id === project.id
          ? {
              ...current,
              saved: saved.revision,
              project: { ...current.project, revision: saved.revision },
              dirty: current.generation !== generation,
            }
          : current,
      );
      if (latest.current.generation === generation)
        localStorage.removeItem("ghostline-character-draft");
      notify("Project saved.");
    } finally {
      setSaving(false);
    }
  };
  const history = (redo = false): void => {
    const current = latest.current,
      source = [...(redo ? current.redo : current.undo)],
      next = source.pop();
    if (!next || !current.project) return;
    setEditor({
      ...current,
      project: next,
      dirty: true,
      generation: current.generation + 1,
      undo: redo ? [...current.undo, current.project] : source,
      redo: redo ? source : [...current.redo, current.project],
    });
  };
  const ids = project ? selectedIds(project, metadata) : new Set<string>();
  const selected = a
    ? [
        ...(a.native_outfit
          ? [
              {
                kind: "native_outfit",
                key: "native_outfit",
                id: `npc-outfit:${project!.template}:${a.native_outfit}`,
              },
            ]
          : []),
        ...Object.entries(a.choices).map(([key, id]) => ({
          kind: "choices",
          key,
          id,
        })),
        ...Object.entries(a.equipment).map(([key, id]) => ({
          kind: "equipment",
          key,
          id,
        })),
        ...(project?.loadout || []).map((id) => ({
          kind: "loadout",
          key: id,
          id,
        })),
      ]
    : [];
  const projectURL = useMemo(
    () =>
      project
        ? URL.createObjectURL(
            new Blob([JSON.stringify(project, null, 2) + "\n"], {
              type: "application/json",
            }),
          )
        : "",
    [project],
  );
  useEffect(
    () => () => {
      if (projectURL) URL.revokeObjectURL(projectURL);
    },
    [projectURL],
  );
  const profile = String(project?.ai.profile || "quest"),
    nativePresets =
      bootstrap?.behaviour.presets.filter(
        (p) =>
          p.id === profile ||
          presetQuery
            .toLowerCase()
            .split(/\s+/)
            .every((w) => JSON.stringify(p).toLowerCase().includes(w)),
      ) || [];
  const control = (
    label: string,
    value: string,
    change: (value: string) => void,
  ) => (
    <label className="grid gap-2 text-sm">
      {label}
      <Input
        value={value}
        onChange={(e) => {
          const nextValue = e.target.value;
          return run(() => change(nextValue));
        }}
      />
    </label>
  );
  return (
    <AppShell
      product="Character builder"
      className="min-h-dvh"
      project={
        <div className="grid gap-1">
          <strong>{project?.name || "Create your character"}</strong>
          <span className="text-xs text-muted">
            {project
              ? `${editor.dirty ? "Unsaved changes" : editor.saved === null ? "Not saved yet" : "Saved · revision " + editor.saved} · ${project.id}`
              : "No project open"}
          </span>
        </div>
      }
      actions={
        <>
          <Button
            disabled={!bootstrap}
            onClick={() => {
              if (discard()) setModal("new");
            }}
          >
            New
          </Button>
          <Button
            onClick={() =>
              run(async () => {
                setProjects(await api("/api/projects"));
                setModal("open");
              })
            }
          >
            Open
          </Button>
          <Button onClick={() => file.current?.click()}>Import</Button>
          <Button
            variant="primary"
            disabled={!project || saving}
            onClick={() => run(save)}
          >
            Save project
          </Button>
          <Button
            aria-label="Local game settings"
            onClick={() => setModal("settings")}
          >
            Settings
          </Button>
        </>
      }
    >
      <main className="grid min-h-0 flex-1 grid-cols-1 lg:grid-cols-[300px_minmax(350px,1fr)_300px]">
        <aside className="space-y-4 border-r border-line p-4">
          <h1 className="text-xl font-semibold">Catalogue</h1>
          <Input
            type="search"
            aria-label="Search catalogue"
            placeholder="Hair, jackets, chrome…"
            value={query}
            onChange={(e) => {
              setQuery(e.target.value);
              setOffset(0);
            }}
          />
          <label className="grid gap-2 text-sm">
            Category
            <Select
              value={category}
              onChange={(e) => {
                setCategory(e.target.value);
                setOffset(0);
              }}
            >
              <option value="">All categories</option>
              {[
                ["Body appearance", "customization"],
                ["Clothing", "clothing"],
                ["Weapons", "loadout"],
              ].map(([title, all]) => (
                <optgroup key={all} label={title!}>
                  <option value={all}>All {title?.toLowerCase()}</option>
                  {bootstrap?.categories
                    .filter((c) =>
                      all === "clothing"
                        ? c.startsWith("Clo_") || c === "npc_outfit"
                        : all === "loadout"
                          ? c.startsWith("Wea_")
                          : !c.startsWith("Clo_") &&
                            !c.startsWith("Wea_") &&
                            c !== "npc_outfit",
                    )
                    .sort()
                    .map((c) => (
                      <option key={c} value={c}>
                        {categoryLabel(c)}
                      </option>
                    ))}
                </optgroup>
              ))}
            </Select>
          </label>
          <div className="flex items-center gap-2 text-xs text-muted">
            <span role="status" className="flex-1">
              {searching
                ? "Searching…"
                : `${results.total ? offset + 1 : 0}–${Math.min(offset + results.entries.length, results.total)} of ${results.total}`}
            </span>
            <Button
              size="sm"
              aria-label="Previous page"
              disabled={!offset}
              onClick={() => setOffset(Math.max(0, offset - 48))}
            >
              ←
            </Button>
            <Button
              size="sm"
              aria-label="Next page"
              disabled={offset + 48 >= results.total}
              onClick={() => setOffset(offset + 48)}
            >
              →
            </Button>
          </div>
          <div className="grid grid-cols-2 gap-3">
            {results.entries.map((entry) => {
              const variants = entry.variants || [entry],
                chosen = variants.find((v) => ids.has(v.id)) || entry,
                media =
                  chosen.media?.find((m) => m.view === "hero") ||
                  chosen.media?.[0],
                unsupported =
                  entry.kind === "equipment" &&
                  !entry.templates &&
                  /_(big|fat)$/.test(project?.template || "");
              return (
                <article
                  key={entry.id}
                  className={`overflow-hidden rounded-lg border bg-panel ${variants.some((v) => ids.has(v.id)) ? "border-accent" : "border-line"}`}
                >
                  <button
                    type="button"
                    className="block w-full text-left disabled:opacity-40"
                    disabled={chosen.resolution !== "resolved" || unsupported}
                    title={chosen.id}
                    onClick={() => run(() => select(chosen))}
                  >
                    {media ? (
                      <img
                        loading="lazy"
                        className="aspect-square w-full object-contain"
                        src={
                          "/media/" +
                          media.path
                            .split("/")
                            .map(encodeURIComponent)
                            .join("/")
                        }
                        alt={displayLabel(chosen)}
                      />
                    ) : (
                      <div className="flex aspect-square items-center justify-center p-3 text-xs text-muted">
                        {entry.kind === "loadout"
                          ? "Loadout weapon"
                          : entry.kind === "npc_clothing"
                            ? `${entry.body_family?.toUpperCase()} · Native outfit`
                            : "Preview unavailable"}
                      </div>
                    )}
                    <div className="space-y-1 p-2">
                      <strong className="block text-xs">
                        {displayLabel(entry)}
                      </strong>
                      <small className="text-muted">
                        {unsupported
                          ? "Requires player / average rig"
                          : variants.length > 1
                            ? `${variants.length} variants`
                            : categoryLabel(entry.category)}
                      </small>
                    </div>
                  </button>
                  {variants.length > 1 && (
                    <Select
                      aria-label={`${displayLabel(entry)} colour / variant`}
                      value={chosen.id}
                      onChange={(e) => {
                        const nextValue = e.target.value;
                        const value = variants.find((v) => v.id === nextValue);
                        if (value) run(() => select(value));
                      }}
                    >
                      {variants.map((v) => (
                        <option key={v.id} value={v.id}>
                          {v.record || v.variant_label || displayLabel(v)}
                        </option>
                      ))}
                    </Select>
                  )}
                </article>
              );
            })}
          </div>
        </aside>
        <section className="min-w-0 space-y-4 p-4">
          <div className="flex flex-wrap gap-2">
            {project?.appearances.map((look) => (
              <Button
                key={look.id}
                variant={
                  look.id === project.active_appearance ? "primary" : "ghost"
                }
                onClick={() =>
                  edit((p) => {
                    p.active_appearance = look.id;
                  })
                }
              >
                {look.name}
              </Button>
            ))}
            <Button
              disabled={!a}
              onClick={() =>
                edit((p) => {
                  const next = structuredClone(appearance(p));
                  let n = p.appearances.length + 1;
                  while (p.appearances.some((v) => v.id === `look_${n}`)) n++;
                  next.id = `look_${n}`;
                  next.name = `Look ${n}`;
                  p.appearances.push(next);
                  p.active_appearance = next.id;
                })
              }
            >
              + Appearance
            </Button>
            <Button
              aria-label="Remove appearance"
              disabled={!project || project.appearances.length < 2}
              onClick={() =>
                edit((p) => {
                  p.appearances = p.appearances.filter(
                    (v) => v.id !== p.active_appearance,
                  );
                  p.active_appearance = p.appearances[0]!.id;
                })
              }
            >
              −
            </Button>
          </div>
          <div className="relative min-h-[500px] rounded-xl border border-line bg-panel">
            <Viewport
              ref={viewport}
              label="Character 3D preview"
              className="h-[65dvh] min-h-[500px]"
              {...(source ? { source } : {})}
              toolbar={false}
              grid={false}
              preserveCamera
              materialAdapter={(model) => {
                model.rotation.y = Math.PI;
                nativeMaterials(model);
              }}
              onError={(e) => notify(e.message, true)}
              onLoad={() => {
                if (camera.focus_head && headBounds.current) {
                  const { center, radius } = headBounds.current;
                  viewport.current?.focus(
                    [-center[0], center[1], -center[2]],
                    radius,
                  );
                }
              }}
            />
            {!source && (
              <div className="pointer-events-none absolute inset-0 flex items-center justify-center p-6 text-center">
                <div>
                  <h2 className="text-2xl font-semibold">
                    {project ? "Preview" : "No character open"}
                  </h2>
                  <p className="mt-3 text-muted">
                    {project
                      ? "Update the preview to see your character."
                      : "Create a character or open a saved project."}
                  </p>
                </div>
              </div>
            )}
          </div>
          {source && previewGeneration !== editor.generation && (
            <p className="text-warning">Preview needs updating</p>
          )}
          <div className="flex flex-wrap gap-2">
            {(["front", "perspective", "back"] as const).map((view) => (
              <Button key={view} onClick={() => viewport.current?.fit(view)}>
                {view === "perspective"
                  ? "¾ view"
                  : view === "front"
                    ? "Front"
                    : "Back"}
              </Button>
            ))}
            {[false, true].map((focus) => (
              <Button
                key={String(focus)}
                aria-pressed={camera.focus_head === focus}
                onClick={() => {
                  setCamera({ ...camera, focus_head: focus, zoom: 1 });
                  if (focus && headBounds.current) {
                    const { center, radius } = headBounds.current;
                    viewport.current?.focus(
                      [-center[0], center[1], -center[2]],
                      radius,
                    );
                  } else viewport.current?.fit("perspective");
                }}
              >
                {focus ? "Face" : "Full body"}
              </Button>
            ))}
            <label className="flex items-center gap-2 text-sm">
              Orbit
              <input
                type="range"
                min="-3.14"
                max="3.14"
                step=".02"
                value={camera.yaw}
                onChange={(e) => {
                  const yaw = Number(e.target.value);
                  viewport.current?.orbit(yaw - camera.yaw);
                  setCamera({ ...camera, yaw });
                }}
              />
            </label>
            <label className="flex items-center gap-2 text-sm">
              Zoom
              <input
                type="range"
                min=".5"
                max="4"
                step=".05"
                value={camera.zoom}
                onChange={(e) => {
                  const zoom = Number(e.target.value);
                  viewport.current?.zoom(camera.zoom / zoom);
                  setCamera({ ...camera, zoom });
                }}
              />
            </label>
          </div>
          <div className="flex flex-wrap items-center gap-3">
            <span className="flex-1 text-sm text-muted" role="status">
              {jobState}
            </span>
            {job && (
              <Button
                onClick={() => run(() => api(`/api/jobs/${job.id}/cancel`, {}))}
              >
                Cancel
              </Button>
            )}
            <Button
              variant="primary"
              disabled={!project || !!job}
              onClick={() => run(() => startJob())}
            >
              Update preview
            </Button>
          </div>
          <p className="text-xs text-muted">
            Catalogue · game{" "}
            {bootstrap?.catalogue.sources.game_version || "unknown"} · local
            previews
          </p>
        </section>
        <aside className="space-y-5 border-l border-line p-4">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <h2 className="text-xl font-semibold">Character</h2>
            <div className="flex gap-1">
              <Button
                size="sm"
                disabled={!editor.undo.length}
                onClick={() => history()}
              >
                Undo
              </Button>
              <Button
                size="sm"
                disabled={!editor.redo.length}
                onClick={() => history(true)}
              >
                Redo
              </Button>
            </div>
          </div>
          {project && a ? (
            <>
              {control("Name", project.name, (v) =>
                edit((p) => {
                  p.name = v;
                }),
              )}
              <p className="text-xs text-muted">{template?.label}</p>
              {template?.head_mode === "transfer" && (
                <>
                  <label className="grid gap-2 text-sm">
                    Native head
                    <Select
                      value={
                        a.native_head ||
                        bootstrap?.native_heads?.[project.template]?.find(
                          (v) => v.default,
                        )?.id ||
                        ""
                      }
                      onChange={(e) =>
                        edit((p) => {
                          appearance(p).native_head = e.target.value;
                        })
                      }
                    >
                      {bootstrap?.native_heads?.[project.template]?.map((o) => (
                        <option key={o.id} value={o.id}>
                          {o.label}
                        </option>
                      ))}
                    </Select>
                  </label>
                  <label className="grid gap-2 text-sm">
                    Skin colour
                    <Select
                      value={a.skin_tone || ""}
                      onChange={(e) =>
                        edit((p) => {
                          appearance(p).skin_tone = e.target.value || null;
                        })
                      }
                    >
                      {[
                        ["", "Match donor head"],
                        ["01_ca_pale", "Pale"],
                        ["02_ca_limestone", "Limestone"],
                        ["03_ca_senna", "Senna"],
                        ["04_ca_almond", "Almond"],
                        ["05_bl_espresso", "Espresso"],
                        ["06_bl_dark", "Dark"],
                      ].map(([v, t]) => (
                        <option key={v} value={v}>
                          {t}
                        </option>
                      ))}
                    </Select>
                  </label>
                </>
              )}
              {control("Appearance name", a.name, (v) =>
                edit((p) => {
                  appearance(p).name = v;
                }),
              )}
              <label className="flex items-center gap-2 text-sm">
                <input
                  type="checkbox"
                  checked={project.refit_clothing === true}
                  onChange={(e) =>
                    edit((p) => {
                      p.refit_clothing = e.target.checked;
                    })
                  }
                />
                Refit clothing (experimental)
              </label>
              <Panel title="Face shape" open>
                {["eyes", "nose", "mouth", "jaw", "ears"].map((shape) => (
                  <label
                    key={shape}
                    className="grid grid-cols-[50px_1fr_20px] items-center gap-2 text-sm"
                  >
                    {shape}
                    <input
                      aria-label={`${shape} shape`}
                      type="range"
                      min="1"
                      max={template?.shape_max || 21}
                      value={a.shapes[shape] || 1}
                      onChange={(e) =>
                        edit((p) => {
                          appearance(p).shapes[shape] = Number(e.target.value);
                        })
                      }
                    />
                    <output>{a.shapes[shape] || 1}</output>
                  </label>
                ))}
              </Panel>
              {["appearance", "loadout"].map((group) => (
                <Panel
                  key={group}
                  title={
                    group === "appearance"
                      ? "Selected appearance"
                      : "Weapons / loadout"
                  }
                  open
                >
                  {selected
                    .filter((row) =>
                      group === "loadout"
                        ? row.kind === "loadout"
                        : row.kind !== "loadout",
                    )
                    .map((row) => (
                      <div
                        key={`${row.kind}:${row.key}`}
                        className="flex items-center gap-2 rounded border border-line p-2"
                      >
                        <span className="min-w-0 flex-1 break-all text-xs">
                          {metadata.choice_entries[row.id]
                            ? displayLabel(metadata.choice_entries[row.id]!)
                            : row.id.split("#").at(-1)}
                        </span>
                        <Button
                          size="sm"
                          aria-label={`Remove ${row.id}`}
                          onClick={() =>
                            edit((p) => {
                              if (row.kind === "native_outfit")
                                appearance(p).native_outfit = null;
                              else if (row.kind === "loadout")
                                p.loadout = p.loadout.filter(
                                  (id) => id !== row.id,
                                );
                              else
                                delete (
                                  row.kind === "choices"
                                    ? appearance(p).choices
                                    : appearance(p).equipment
                                )[row.key];
                            })
                          }
                        >
                          ×
                        </Button>
                      </div>
                    ))}
                </Panel>
              ))}
              <Panel title="NPC behaviour">
                <Input
                  type="search"
                  aria-label="Find behaviour presets"
                  placeholder="Netrunner, melee, faction…"
                  value={presetQuery}
                  onChange={(e) => setPresetQuery(e.target.value)}
                />
                <label className="grid gap-2 text-sm">
                  Behaviour preset
                  <Select
                    value={profile}
                    onChange={(e) =>
                      edit((p) => {
                        p.ai.profile = e.target.value;
                        const preset = bootstrap?.behaviour.presets.find(
                          (v) => v.id === e.target.value,
                        );
                        for (const [key, native] of aiFields)
                          p.ai[key] = preset?.values[native] || "";
                      })
                    }
                  >
                    <optgroup label="Authoring presets">
                      {[
                        ["quest", "Quest NPC"],
                        ["combat", "Combat NPC"],
                        ["cyberpsycho_netrunner", "Cyberpsycho · netrunner"],
                      ].map(([v, t]) => (
                        <option key={v} value={v}>
                          {t}
                        </option>
                      ))}
                    </optgroup>
                    <optgroup label="Native presets">
                      {nativePresets.map((p) => (
                        <option key={p.id} value={p.id}>
                          {Object.values(p.values).join(" · ")}
                        </option>
                      ))}
                    </optgroup>
                  </Select>
                </label>
                <small className="text-muted">
                  {nativePresets.length} native presets
                </small>
                <Panel title="Advanced behaviour">
                  {aiFields.map(([key, native, title]) => (
                    <label key={key} className="grid gap-2 text-sm">
                      {title}
                      <Select
                        value={String(project.ai[key] || "")}
                        onChange={(e) =>
                          edit((p) => {
                            p.ai[key] = e.target.value;
                          })
                        }
                      >
                        <option value="">Inherit preset</option>
                        {project.ai[key] &&
                          !bootstrap?.behaviour.options[native]?.includes(
                            String(project.ai[key]),
                          ) && <option>{String(project.ai[key])}</option>}
                        {bootstrap?.behaviour.options[native]?.map((v) => (
                          <option key={v}>{v}</option>
                        ))}
                      </Select>
                    </label>
                  ))}
                </Panel>
                {(["power_level", "health_multiplier"] as const).map((key) => (
                  <label key={key} className="grid gap-2 text-sm">
                    {key === "power_level"
                      ? "Power level"
                      : "Health multiplier"}
                    <Input
                      type="number"
                      min={key === "power_level" ? 1 : 0.01}
                      step={key === "power_level" ? 1 : 0.1}
                      value={project.ai[key] ?? ""}
                      onChange={(e) =>
                        edit((p) => {
                          p.ai[key] = e.target.value
                            ? Number(e.target.value)
                            : null;
                        })
                      }
                    />
                  </label>
                ))}
              </Panel>
              <Panel title="Export identity">
                {(["record", "voice_tag", "affiliation"] as const).map(
                  (key) => (
                    <div key={key}>
                      {control(
                        key.replaceAll("_", " "),
                        String(project.identity[key] || ""),
                        (v) =>
                          edit((p) => {
                            p.identity[key] = v;
                          }),
                      )}
                    </div>
                  ),
                )}
                {control("Depot namespace", project.namespace, (v) =>
                  edit((p) => {
                    p.namespace = v;
                  }),
                )}
              </Panel>
              <div className="flex flex-wrap gap-2">
                <Button
                  onClick={() =>
                    run(async () => {
                      const result = await api<{
                        ok: boolean;
                        errors: string[];
                        warnings: string[];
                      }>("/api/validate", project);
                      notify(
                        [
                          result.ok ? "No issues found." : "Validation failed.",
                          ...result.errors,
                          ...result.warnings,
                        ].join("\n"),
                        !result.ok,
                      );
                    })
                  }
                >
                  Validate
                </Button>
                <a
                  className="rounded border border-line px-3 py-2 text-sm"
                  href={projectURL}
                  download={project.id + ".character.json"}
                >
                  Project JSON
                </a>
                <Button
                  disabled={!!job}
                  onClick={() => run(() => startJob(true))}
                >
                  Export character
                </Button>
              </div>
            </>
          ) : (
            <p className="text-muted">Create or open a character to edit it.</p>
          )}
          <div
            role={bad ? "alert" : "status"}
            className={`whitespace-pre-wrap text-sm ${bad ? "text-danger" : "text-muted"}`}
          >
            {message}
          </div>
          {downloads.map((d) => (
            <a
              key={d.href}
              className="block text-sm text-accent"
              href={d.href}
              download
            >
              {d.label}
            </a>
          ))}
          {draft && (
            <Button
              onClick={() =>
                run(async () => {
                  if (!discard()) return;
                  const value = JSON.parse(
                    localStorage.getItem("ghostline-character-draft") || "null",
                  ) as Project;
                  prepared(
                    await api("/api/check-project", value),
                    value.revision || null,
                    true,
                  );
                  setDraft(false);
                })
              }
            >
              Restore unsaved draft
            </Button>
          )}
        </aside>
      </main>
      {modal === "new" && (
        <Modal title="New character" close={() => setModal(null)}>
          <form
            className="space-y-4"
            onSubmit={(e) => {
              e.preventDefault();
              run(async () => {
                if (!discard()) return;
                load(
                  await api("/api/projects/new", {
                    id: newId,
                    template: newTemplate,
                  }),
                );
                setModal(null);
                notify("");
              });
            }}
          >
            <label className="grid gap-2 text-sm">
              Project ID
              <Input
                required
                pattern="[a-zA-Z0-9_-]{1,80}"
                value={newId}
                onChange={(e) => setNewId(e.target.value)}
                placeholder="my_character"
              />
            </label>
            <label className="grid gap-2 text-sm">
              Body template
              <Select
                value={newTemplate}
                onChange={(e) => setNewTemplate(e.target.value)}
              >
                {bootstrap?.templates.map((t) => (
                  <option key={t.id} value={t.id}>
                    {t.label}
                  </option>
                ))}
              </Select>
            </label>
            <Button type="submit" variant="primary">
              Create character
            </Button>
          </form>
        </Modal>
      )}
      {modal === "open" && (
        <Modal title="Your characters" close={() => setModal(null)}>
          <div className="space-y-3">
            {projects.map((p) => (
              <Button
                key={p.id || p.file}
                className="flex w-full justify-between"
                disabled={!!p.error}
                onClick={() =>
                  run(async () => {
                    if (!discard()) return;
                    prepared(
                      await api("/api/projects/" + encodeURIComponent(p.id)),
                      p.revision,
                    );
                    setModal(null);
                  })
                }
              >
                <span>{p.name || p.file}</span>
                <small>{p.error || p.template}</small>
              </Button>
            ))}
          </div>
        </Modal>
      )}
      {modal === "settings" && (
        <Modal title="Game resources" close={() => setModal(null)}>
          <form
            className="space-y-4"
            onSubmit={(e) => {
              e.preventDefault();
              run(async () => {
                const value = {
                  game: settings.game || null,
                  schema: settings.schema || null,
                };
                await api("/api/settings", value);
                if (bootstrap) setBootstrap({ ...bootstrap, settings: value });
                setModal(null);
                notify("Local resources configured.");
              });
            }}
          >
            {control("Cyberpunk 2077 directory", settings.game, (v) =>
              setSettings({ ...settings, game: v }),
            )}
            {control("Reflection override (optional)", settings.schema, (v) =>
              setSettings({ ...settings, schema: v }),
            )}
            <Button type="submit" variant="primary">
              Use resources
            </Button>
          </form>
        </Modal>
      )}
      <input
        ref={file}
        type="file"
        accept=".json"
        className="hidden"
        onChange={(e) => {
          const imported = e.target.files?.[0];
          e.target.value = "";
          if (imported && discard())
            run(async () =>
              prepared(
                await api(
                  "/api/check-project",
                  JSON.parse(await imported.text()),
                ),
                null,
                true,
              ),
            );
        }}
      />
    </AppShell>
  );
}
function Panel({
  title,
  children,
  open = false,
}: {
  title: string;
  children: ReactNode;
  open?: boolean;
}) {
  return (
    <details open={open} className="rounded-lg border border-line p-3">
      <summary className="text-sm font-semibold">{title}</summary>
      <div className="mt-4 space-y-4">{children}</div>
    </details>
  );
}
createRoot(document.getElementById("root")!).render(<CharacterBuilder />);
