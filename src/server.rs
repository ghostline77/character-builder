use std::{
    collections::BTreeMap,
    fs,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
};

use anyhow::{Context, Result};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path as RoutePath, Query, State},
    http::{StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    catalogue::{self, Catalogue},
    native::{self, PreviewOptions, Session},
    project::{self, Project},
    templates,
};

macro_rules! ensure {
    ($condition:expr, $($message:tt)*) => {
        if !$condition { return Err(anyhow::anyhow!($($message)*).into()); }
    };
}

#[derive(Clone)]
struct App {
    catalogue: Arc<Catalogue>,
    projects: PathBuf,
    work: PathBuf,
    jobs: Arc<Mutex<BTreeMap<u64, Value>>>,
    next: Arc<AtomicU64>,
    cancellations: Arc<Mutex<BTreeMap<u64, Arc<AtomicBool>>>>,
    sender: mpsc::Sender<Work>,
    settings: Arc<Mutex<Settings>>,
    save_lock: Arc<Mutex<()>>,
}

#[derive(Clone, Deserialize, serde::Serialize, PartialEq)]
struct Settings {
    game: Option<PathBuf>,
    schema: Option<PathBuf>,
}

struct Work {
    id: u64,
    project: Project,
    export: bool,
    camera: PreviewOptions,
    cancelled: Arc<AtomicBool>,
}

struct ApiError(anyhow::Error);
impl<E: Into<anyhow::Error>> From<E> for ApiError {
    fn from(error: E) -> Self {
        Self(error.into())
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":format!("{:#}",self.0)})),
        )
            .into_response()
    }
}
type Api<T> = std::result::Result<T, ApiError>;

pub async fn serve(
    catalogue: PathBuf,
    projects: PathBuf,
    work: PathBuf,
    game: Option<PathBuf>,
    schema: Option<PathBuf>,
    port: u16,
) -> Result<()> {
    fs::create_dir_all(&projects)?;
    fs::create_dir_all(&work)?;
    let catalogue = Arc::new(Catalogue::load(&catalogue)?);
    let (sender, receiver) = mpsc::channel::<Work>();
    let app = App {
        catalogue,
        projects: projects.canonicalize()?,
        work: work.canonicalize()?,
        jobs: Arc::new(Mutex::new(BTreeMap::new())),
        next: Arc::new(AtomicU64::new(
            fs::read_dir(work.join("jobs"))
                .ok()
                .into_iter()
                .flatten()
                .filter_map(|entry| entry.ok()?.file_name().to_str()?.parse::<u64>().ok())
                .max()
                .unwrap_or(0)
                .saturating_add(1),
        )),
        cancellations: Arc::new(Mutex::new(BTreeMap::new())),
        sender,
        settings: Arc::new(Mutex::new(Settings { game, schema })),
        save_lock: Arc::new(Mutex::new(())),
    };
    let worker = app.clone();
    std::thread::Builder::new()
        .name("character-preview".into())
        .spawn(move || {
            let mut session: Option<(Settings, Session)> = None;
            while let Ok(job) = receiver.recv() {
                let cancelled = || job.cancelled.load(Ordering::Acquire);
                let result = (|| -> Result<Value> {
                    ensure!(!cancelled(), "job superseded or cancelled");
                    update_job(&worker, job.id, "running", Value::Null);
                    let settings = worker
                        .settings
                        .lock()
                        .map_err(|_| anyhow::anyhow!("settings lock failed"))?
                        .clone();
                    let game = settings
                        .game
                        .as_ref()
                        .context("configure the local game directory")?;
                    let schema = settings.schema.as_deref();
                    let output = worker.work.join("jobs").join(job.id.to_string());
                    if job.export {
                        native::build(
                            &job.project,
                            &worker.catalogue,
                            game,
                            schema,
                            &output,
                            true,
                            cancelled,
                        )
                    } else {
                        if session
                            .as_ref()
                            .is_none_or(|(active, _)| *active != settings)
                        {
                            session = Some((settings.clone(), Session::open(game, schema)?));
                        }
                        let result = session
                            .as_mut()
                            .context("preview session is absent")?
                            .1
                            .preview(
                                &job.project,
                                &worker.catalogue,
                                &output,
                                &job.camera,
                                cancelled,
                            )?;
                        ensure!(!cancelled(), "preview superseded or cancelled");
                        Ok(result)
                    }
                })();
                match result {
                    Ok(value) => update_job(&worker, job.id, "complete", value),
                    Err(error) => update_job(
                        &worker,
                        job.id,
                        if cancelled() { "cancelled" } else { "failed" },
                        json!({"error":format!("{error:#}")}),
                    ),
                }
            }
        })?;
    let router = Router::new()
        .route(
            "/",
            get(|| async { Html(include_str!("../ui/dist/index.html")) }),
        )
        .route(
            "/app.js",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
                    include_str!("../ui/dist/app.js"),
                )
            }),
        )
        .route(
            "/style.css",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
                    include_str!("../ui/dist/style.css"),
                )
            }),
        )
        .route("/api/bootstrap", get(bootstrap))
        .route("/api/catalogue", get(search))
        .route("/api/selection", get(selection))
        .route("/api/projects", get(list_projects).post(save))
        .route("/api/projects/new", post(new_project))
        .route("/api/projects/{id}", get(open))
        .route("/api/validate", post(validate))
        .route("/api/check-project", post(check_project))
        .route("/api/project-selections", post(project_selections))
        .route("/api/choices/select", post(select_choice))
        .route("/api/jobs", post(start_job))
        .route("/api/jobs/{id}", get(job))
        .route("/api/jobs/{id}/cancel", post(cancel))
        .route("/api/settings", post(settings))
        .route("/media/{*path}", get(media))
        .route("/output/{id}/{*path}", get(output))
        .layer(DefaultBodyLimit::max(4 * 1024 * 1024))
        .with_state(app);
    let listener =
        tokio::net::TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port))
            .await?;
    println!("Character builder: http://{}", listener.local_addr()?);
    axum::serve(listener, router).await?;
    Ok(())
}

fn update_job(app: &App, id: u64, state: &str, result: Value) {
    if let Ok(mut jobs) = app.jobs.lock()
        && let Some(job) = jobs.get_mut(&id)
    {
        job["state"] = json!(state);
        job["result"] = result;
    }
}

async fn bootstrap(State(app): State<App>) -> Api<Json<Value>> {
    let native_heads: std::collections::BTreeMap<_, _> = templates::TEMPLATES
        .iter()
        .map(|&t| Ok((t.id, crate::frames::options(t)?)))
        .collect::<anyhow::Result<_>>()?;
    Ok(Json(
        json!({"templates":templates::TEMPLATES,"native_heads":native_heads,"behaviour":crate::ai::catalogue()?,"catalogue":app.catalogue.pin,"categories":app.catalogue.categories(),"settings":*app.settings.lock().map_err(|_| anyhow::anyhow!("settings unavailable"))?}),
    ))
}

#[derive(Default, Deserialize)]
struct Search {
    #[serde(default)]
    template: String,
    #[serde(default)]
    q: String,
    #[serde(default)]
    category: String,
    #[serde(default)]
    frame: String,
    #[serde(default)]
    offset: usize,
}
async fn search(State(app): State<App>, Query(query): Query<Search>) -> Json<Value> {
    Json(app.catalogue.search(
        &query.q,
        &query.category,
        &query.frame,
        &query.template,
        query.offset,
        48,
    ))
}
#[derive(Deserialize)]
struct SelectionQuery {
    id: String,
}
async fn selection(
    State(app): State<App>,
    Query(query): Query<SelectionQuery>,
) -> Api<Json<Value>> {
    Ok(Json(
        app.catalogue
            .selections
            .get(&query.id)
            .context("selection is absent")?
            .clone(),
    ))
}
async fn list_projects(State(app): State<App>) -> Api<Json<Value>> {
    let mut projects = Vec::new();
    for entry in fs::read_dir(&app.projects)? {
        let path = entry?.path();
        if path.extension().is_some_and(|e| e == "json") {
            match project::read(&path) {
                Ok(project) => projects.push(json!({"id":project.id,"name":project.name,"template":project.template,"revision":project.revision})),
                Err(error) => projects.push(json!({"file":path.file_name(),"error":format!("{error:#}")})),
            }
        }
    }
    Ok(Json(json!(projects)))
}
#[derive(Deserialize)]
struct New {
    id: String,
    template: String,
}
async fn new_project(State(app): State<App>, Json(request): Json<New>) -> Api<Json<Project>> {
    let mut project = Project::new(&request.id, &request.template)?;
    app.catalogue.defaults(&mut project)?;
    Ok(Json(project))
}
async fn open(
    State(app): State<App>,
    RoutePath(id): RoutePath<String>,
) -> Api<Json<crate::catalogue::LoadedProject>> {
    project::safe_id(&id)?;
    Ok(Json(app.catalogue.prepare_project(project::read(
        &app.projects.join(format!("{id}.character.json")),
    )?)?))
}
#[derive(Deserialize)]
struct Save {
    project: Project,
    expected_revision: Option<u64>,
}
async fn save(State(app): State<App>, Json(mut request): Json<Save>) -> Api<Json<Project>> {
    request.project.check()?;
    let _guard = app
        .save_lock
        .lock()
        .map_err(|_| anyhow::anyhow!("project store is unavailable"))?;
    let path = app
        .projects
        .join(format!("{}.character.json", request.project.id));
    if path.exists() {
        let current = project::read(&path)?;
        ensure!(
            request.expected_revision == Some(current.revision),
            "project changed on disk; reopen it before saving"
        );
        request.project.revision = current
            .revision
            .checked_add(1)
            .context("revision overflow")?;
    } else {
        ensure!(
            request.expected_revision.is_none(),
            "saved project no longer exists"
        );
        request.project.revision = 1;
    }
    project::write(&path, &request.project)?;
    Ok(Json(request.project))
}
async fn validate(State(app): State<App>, Json(project): Json<Project>) -> Json<Value> {
    Json(json!(app.catalogue.validate(&project)))
}
async fn check_project(
    State(app): State<App>,
    Json(project): Json<Project>,
) -> Api<Json<crate::catalogue::LoadedProject>> {
    Ok(Json(app.catalogue.prepare_project(project)?))
}

#[derive(Deserialize)]
struct SelectChoice {
    project: Project,
    choice: String,
}
async fn select_choice(
    State(app): State<App>,
    Json(mut request): Json<SelectChoice>,
) -> Api<Json<Project>> {
    app.catalogue
        .select_choice(&mut request.project, &request.choice)?;
    Ok(Json(request.project))
}
#[derive(Deserialize)]
struct ProjectRequest {
    project: Project,
}
async fn project_selections(
    State(app): State<App>,
    Json(request): Json<ProjectRequest>,
) -> Api<Json<Value>> {
    request.project.check()?;
    let mut selections = BTreeMap::new();
    let mut entries = BTreeMap::new();
    for id in request.project.appearance().choices.values() {
        if let Some(choice) = app.catalogue.choices.get(id) {
            selections.insert(id, choice["selection"].clone());
            if let Some(control) = choice["control"]
                .as_str()
                .and_then(|id| app.catalogue.controls.get(id))
            {
                entries.insert(id.clone(),json!({"label":choice["label"],"category":control["native"]["ui_slot"],"control_name":control["native"]["name"]}));
            }
        }
    }
    for id in request
        .project
        .appearance()
        .equipment
        .values()
        .chain(&request.project.loadout)
    {
        if let Some(entry) = app.catalogue.entries.iter().find(|e| e["id"] == *id) {
            entries.insert(id.clone(), entry.clone());
        }
    }
    if let Some(id) = &request.project.appearance().native_outfit {
        let key = format!("npc-outfit:{}:{id}", request.project.template);
        if let Some(entry) = app.catalogue.entries.iter().find(|e| e["id"] == key) {
            entries.insert(key, entry.clone());
        }
    }
    Ok(Json(
        json!({"choice_selections":selections,"choice_entries":entries}),
    ))
}
#[derive(Deserialize)]
struct Start {
    project: Project,
    #[serde(default)]
    export: bool,
    #[serde(default)]
    camera: PreviewOptions,
}
async fn start_job(State(app): State<App>, Json(request): Json<Start>) -> Api<Json<Value>> {
    request.project.check()?;
    let validation = app.catalogue.validate(&request.project);
    ensure!(validation.ok, "{}", validation.errors.join("; "));
    let id = app.next.fetch_add(1, Ordering::Relaxed);
    let cancelled = Arc::new(AtomicBool::new(false));
    {
        let jobs = app
            .jobs
            .lock()
            .map_err(|_| anyhow::anyhow!("job store is unavailable"))?;
        let mut cancellations = app
            .cancellations
            .lock()
            .map_err(|_| anyhow::anyhow!("cancellation store is unavailable"))?;
        if !request.export {
            for (previous_id, previous) in jobs.iter().filter(|(_, job)| {
                job["project"] == request.project.id
                    && job["kind"] == "preview"
                    && (job["state"] == "queued" || job["state"] == "running")
            }) {
                let _ = previous;
                if let Some(flag) = cancellations.get(previous_id) {
                    flag.store(true, Ordering::Release);
                }
            }
        }
        cancellations.insert(id, Arc::clone(&cancelled));
    }
    let job = json!({"id":id,"project":request.project.id,"revision":request.project.revision,"appearance":request.project.active_appearance,"state":"queued","kind":if request.export {"export"} else {"preview"},"result":null});
    app.jobs
        .lock()
        .map_err(|_| anyhow::anyhow!("job store is unavailable"))?
        .insert(id, job.clone());
    app.sender.send(Work {
        id,
        project: request.project,
        export: request.export,
        camera: request.camera,
        cancelled,
    })?;
    Ok(Json(job))
}
async fn job(State(app): State<App>, RoutePath(id): RoutePath<u64>) -> Api<Json<Value>> {
    Ok(Json(
        app.jobs
            .lock()
            .map_err(|_| anyhow::anyhow!("job store is unavailable"))?
            .get(&id)
            .context("job is absent")?
            .clone(),
    ))
}
async fn cancel(State(app): State<App>, RoutePath(id): RoutePath<u64>) -> Api<Json<Value>> {
    let jobs = app
        .jobs
        .lock()
        .map_err(|_| anyhow::anyhow!("job store is unavailable"))?;
    ensure!(jobs.contains_key(&id), "job is absent");
    app.cancellations
        .lock()
        .map_err(|_| anyhow::anyhow!("cancellation store is unavailable"))?
        .get(&id)
        .context("job cancellation is absent")?
        .store(true, Ordering::Release);
    Ok(Json(json!({"cancelled":id})))
}
async fn settings(State(app): State<App>, Json(settings): Json<Settings>) -> Api<Json<Value>> {
    if let Some(game) = &settings.game {
        ensure!(
            game.join("archive/pc/content").is_dir(),
            "game directory has no archive/pc/content"
        );
    }
    if let Some(schema) = &settings.schema {
        ensure!(schema.is_file(), "schema file is absent");
    }
    *app.settings
        .lock()
        .map_err(|_| anyhow::anyhow!("settings unavailable"))? = settings;
    Ok(Json(json!({"ok":true})))
}
fn serve_file(root: &Path, name: &str) -> Api<Response> {
    let path = catalogue::relative(root, name)?;
    let mime = match path.extension().and_then(|s| s.to_str()).unwrap_or("") {
        "png" => "image/png",
        "glb" => "model/gltf-binary",
        "jpg" | "jpeg" => "image/jpeg",
        "json" => "application/json",
        "archive" => "application/octet-stream",
        "yaml" => "text/yaml",
        _ => "application/octet-stream",
    };
    Ok((
        [
            (header::CONTENT_TYPE, mime),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        fs::read(path)?,
    )
        .into_response())
}
async fn media(State(app): State<App>, RoutePath(path): RoutePath<String>) -> Api<Response> {
    serve_file(&app.catalogue.root, &path)
}
async fn output(
    State(app): State<App>,
    RoutePath((id, path)): RoutePath<(u64, String)>,
) -> Api<Response> {
    serve_file(&app.work.join("jobs").join(id.to_string()), &path)
}
