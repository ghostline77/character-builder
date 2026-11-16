mod ai;
mod assembly;
mod catalogue;
mod cyberware_review;
mod fit_review;
mod fitting;
mod frames;
mod native;
mod parts;
mod poses;
mod project;
mod server;
mod templates;
mod tweaks;

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Inspect and preview a native cyberware assembly on a target body.
    ReviewCyberware {
        #[arg(long)]
        project: Option<PathBuf>,
        #[arg(long, default_value = "mod\\ghostline_refits\\male_big")]
        depot: String,
        #[arg(long)]
        overlay: Vec<PathBuf>,
        #[arg(long)]
        fit: bool,
        #[arg(long, env = "GHOSTLINE_ITEMS_CATALOGUE", default_value = "catalogue")]
        catalogue: PathBuf,
        #[arg(long)]
        game: PathBuf,
        #[arg(long)]
        schema: Option<PathBuf>,
        #[arg(long, default_value = "male_big")]
        template: String,
        #[arg(long)]
        entity: String,
        #[arg(long)]
        out: PathBuf,
    },
    /// Render native body clips on an authored character without garment refitting.
    RenderAnimations {
        project: PathBuf,
        #[arg(long, env = "GHOSTLINE_ITEMS_CATALOGUE", default_value = "catalogue")]
        catalogue: PathBuf,
        #[arg(long)]
        game: PathBuf,
        #[arg(long)]
        schema: Option<PathBuf>,
        #[arg(long)]
        clips: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    /// Check independent catalogue garments across body rigs and skeletal poses.
    ReviewFits {
        #[arg(long, env = "GHOSTLINE_ITEMS_CATALOGUE", default_value = "catalogue")]
        catalogue: PathBuf,
        #[arg(long)]
        game: PathBuf,
        #[arg(long)]
        schema: Option<PathBuf>,
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        project: Vec<PathBuf>,
        #[arg(long, default_value_t = 2)]
        samples_per_rig: usize,
        #[arg(long)]
        unfitted: bool,
        /// Draw-only component=decimal-mask diagnostic; preserves native rig setup.
        #[arg(long)]
        mask_component: Vec<String>,
    },
    /// Render and export a native outfit on each NPC anatomical family.
    ReviewWardrobe {
        #[arg(long, env = "GHOSTLINE_ITEMS_CATALOGUE", default_value = "catalogue")]
        catalogue: PathBuf,
        #[arg(long)]
        game: PathBuf,
        #[arg(long)]
        schema: Option<PathBuf>,
        #[arg(long)]
        out: PathBuf,
    },
    /// Rebuild native NPC behaviour choices and presets for the bundled UI.
    IndexBehaviour {
        #[arg(long)]
        game: PathBuf,
        #[arg(long)]
        schema: Option<PathBuf>,
        #[arg(long, default_value = "data/behaviour.json")]
        out: PathBuf,
    },
    /// Render every body frame with baseline and alternate native heads.
    Review {
        #[arg(long, env = "GHOSTLINE_ITEMS_CATALOGUE", default_value = "catalogue")]
        catalogue: PathBuf,
        #[arg(long)]
        game: PathBuf,
        #[arg(long)]
        schema: Option<PathBuf>,
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        export: bool,
    },
    /// Open the local character authoring application.
    Serve {
        #[arg(long, env = "GHOSTLINE_ITEMS_CATALOGUE", default_value = "catalogue")]
        catalogue: PathBuf,
        #[arg(long, default_value = "work/projects")]
        projects: PathBuf,
        #[arg(long, default_value = "work")]
        work: PathBuf,
        #[arg(long)]
        game: Option<PathBuf>,
        #[arg(long)]
        schema: Option<PathBuf>,
        #[arg(long, default_value_t = 8787)]
        port: u16,
    },
    /// Create a portable character project.
    New {
        #[arg(long)]
        id: String,
        #[arg(long, default_value = "female_average")]
        template: String,
        #[arg(long)]
        out: PathBuf,
        #[arg(long, env = "GHOSTLINE_ITEMS_CATALOGUE", default_value = "catalogue")]
        catalogue: PathBuf,
    },
    /// Validate a saved character against the selected catalogue.
    Validate {
        project: PathBuf,
        #[arg(long, env = "GHOSTLINE_ITEMS_CATALOGUE", default_value = "catalogue")]
        catalogue: PathBuf,
    },
    /// Resolve, render or export an authored character with native libraries.
    Build {
        project: PathBuf,
        #[arg(long, env = "GHOSTLINE_ITEMS_CATALOGUE", default_value = "catalogue")]
        catalogue: PathBuf,
        #[arg(long)]
        game: PathBuf,
        #[arg(long)]
        schema: Option<PathBuf>,
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        export: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse().command {
        Command::ReviewCyberware {
            project,
            depot,
            overlay,
            fit,
            catalogue,
            game,
            schema,
            template,
            entity,
            out,
        } => {
            cyberware_review::run(
                &catalogue::Catalogue::load(&catalogue)?,
                &game,
                schema.as_deref(),
                &out,
                cyberware_review::Options {
                    project: project.as_deref(),
                    depot: &depot,
                    template: &template,
                    entity: &entity,
                    fit,
                    overlays: &overlay,
                },
            )?;
        }
        Command::RenderAnimations {
            project,
            catalogue,
            game,
            schema,
            clips,
            out,
        } => {
            let project = project::read(&project)?;
            anyhow::ensure!(
                !project.refit_clothing,
                "Animation preview requires refit_clothing=false"
            );
            let catalogue = catalogue::Catalogue::load(&catalogue)?;
            native::Session::open(&game, schema.as_deref())?
                .render_animation_clips(&project, &catalogue, &clips, &out)?;
        }
        Command::ReviewFits {
            catalogue,
            game,
            schema,
            out,
            project,
            samples_per_rig,
            unfitted,
            mask_component,
        } => fit_review::run(
            &catalogue::Catalogue::load(&catalogue)?,
            &game,
            schema.as_deref(),
            &out,
            fit_review::Options {
                inputs: &project,
                samples_per_rig,
                unfitted,
                mask_components: &mask_component,
            },
        )?,
        Command::ReviewWardrobe {
            catalogue,
            game,
            schema,
            out,
        } => native::review_wardrobe(
            &catalogue::Catalogue::load(&catalogue)?,
            &game,
            schema.as_deref(),
            &out,
        )?,
        Command::IndexBehaviour { game, schema, out } => {
            ai::generate(&game, schema.as_deref(), &out)?
        }
        Command::Review {
            catalogue,
            game,
            schema,
            out,
            export,
        } => {
            let catalogue = catalogue::Catalogue::load(&catalogue)?;
            native::review(&catalogue, &game, schema.as_deref(), &out, export)?;
        }
        Command::Serve {
            catalogue,
            projects,
            work,
            game,
            schema,
            port,
        } => {
            server::serve(catalogue, projects, work, game, schema, port).await?;
        }
        Command::New {
            id,
            template,
            out,
            catalogue,
        } => {
            let mut project = project::Project::new(&id, &template)?;
            catalogue::Catalogue::load(&catalogue)?.defaults(&mut project)?;
            project::write(&out, &project)?;
            println!("{}", out.display());
        }
        Command::Validate { project, catalogue } => {
            let project = project::read(&project)?;
            let catalogue = catalogue::Catalogue::load(&catalogue)?;
            let report = catalogue.validate(&project);
            println!("{}", serde_json::to_string_pretty(&report)?);
            anyhow::ensure!(report.ok, "character validation failed");
        }
        Command::Build {
            project,
            catalogue,
            game,
            schema,
            out,
            export,
        } => {
            let project = project::read(&project)?;
            let catalogue = catalogue::Catalogue::load(&catalogue)?;
            let result = native::build(
                &project,
                &catalogue,
                &game,
                schema.as_deref(),
                &out,
                export,
                || false,
            )?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
    }
    Ok(())
}
