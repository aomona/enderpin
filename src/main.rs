use std::{
    collections::BTreeMap,
    io::{self, IsTerminal},
    path::PathBuf,
};

use anyhow::{Context, Result, bail, ensure};
use clap::{Args, Parser, Subcommand, ValueEnum};
use dialoguer::{Confirm, MultiSelect, Select};
use enderpin::{
    Kind, Manifest, Package, Side, SyncOptions, Workspace,
    model::{Lockfile, hash_bytes, identifier},
    storage::Cache,
};
use serde::Serialize;

#[derive(Parser)]
#[command(version, about = "Reproducible Minecraft client and server workspaces")]
struct Cli {
    #[arg(short = 'C', long, global = true, default_value = ".")]
    directory: PathBuf,
    #[arg(long, global = true)]
    cache_dir: Option<PathBuf>,
    #[arg(long, global = true, value_enum, default_value = "client")]
    target: Scope,
    #[arg(long, global = true)]
    json: bool,
    #[arg(long, global = true)]
    no_interactive: bool,
    /// Accept defaults for optional dependency prompts.
    #[arg(short, long, global = true)]
    yes: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Copy, ValueEnum)]
enum Scope {
    Client,
    Server,
    All,
}
impl Scope {
    fn sides(self) -> Vec<Side> {
        match self {
            Self::Client => vec![Side::Client],
            Self::Server => vec![Side::Server],
            Self::All => Side::ALL.to_vec(),
        }
    }
    fn single(self) -> Result<Side> {
        match self {
            Self::Client => Ok(Side::Client),
            Self::Server => Ok(Side::Server),
            Self::All => bail!("search requires --target client or server"),
        }
    }
}

#[derive(Subcommand)]
enum Command {
    /// Create a shared client/server workspace.
    Init {
        #[arg(long)]
        minecraft: String,
    },
    /// Add a Modrinth project or a public HTTPS file URL and synchronize.
    Add(Add),
    /// Remove a direct package; retain dependencies still needed by other packages.
    Remove { name: String },
    /// Search Modrinth, optionally choose a result and install it.
    Search {
        #[arg(default_value = "")]
        query: String,
        #[arg(long, value_enum, default_value = "mod")]
        kind: Kind,
        #[arg(long, default_value_t = 0)]
        offset: u64,
    },
    /// Reconcile files with the configuration, preserving existing locked versions.
    Sync(Sync),
    /// Resolve and write the lock without changing game directories.
    Lock {
        #[arg(long)]
        offline: bool,
    },
    /// Update Modrinth packages; URL packages retain their pinned content.
    Update,
    /// Explicitly restore modified managed files from the lock.
    Restore {
        #[arg(long)]
        offline: bool,
        #[arg(long)]
        no_ignore: bool,
    },
    /// Show locked packages and whether each target's lock matches its configuration.
    List,
}

#[derive(Args)]
struct Add {
    source: String,
    #[arg(long)]
    name: Option<String>,
    /// Required for direct URLs; inferred for Modrinth projects.
    #[arg(long, value_enum)]
    kind: Option<Kind>,
    /// An exact Modrinth version ID.
    #[arg(long)]
    version_id: Option<String>,
    #[arg(long)]
    prerelease: bool,
    #[arg(long, value_delimiter = ',')]
    optional: Vec<String>,
    #[arg(long)]
    filename: Option<String>,
}

#[derive(Args)]
struct Sync {
    #[arg(long)]
    locked: bool,
    #[arg(long)]
    offline: bool,
    #[arg(long)]
    no_ignore: bool,
}

impl Cli {
    fn interactive(&self) -> bool {
        !self.no_interactive
            && !self.json
            && io::stdin().is_terminal()
            && io::stdout().is_terminal()
            && io::stderr().is_terminal()
    }
    fn emit(&self, value: &impl Serialize, message: impl AsRef<str>) -> Result<()> {
        if self.json {
            println!("{}", serde_json::to_string_pretty(value)?);
        } else {
            println!("{}", message.as_ref());
        }
        Ok(())
    }
}

fn clean(text: &str) -> String {
    text.chars()
        .filter(|c| {
            !c.is_control()
                && !('\u{202a}'..='\u{202e}').contains(c)
                && !('\u{2066}'..='\u{2069}').contains(c)
        })
        .collect()
}

fn packages_mut(manifest: &mut Manifest, scope: Scope) -> &mut BTreeMap<String, Package> {
    match scope {
        Scope::All => &mut manifest.common,
        Scope::Client => &mut manifest.client.packages,
        Scope::Server => &mut manifest.server.packages,
    }
}

fn apply(
    cli: &Cli,
    ws: &mut Workspace,
    manifest: Manifest,
    plan: Lockfile,
    options: SyncOptions,
) -> Result<()> {
    let report = ws.apply(manifest, plan, &cli.target.sides(), options, |p| {
        if !cli.json {
            eprintln!("{}: {} {}", p.target.name(), p.action, clean(&p.package));
        }
    })?;
    let message = format!(
        "{} added/replaced, {} removed, {} unchanged{}",
        report.added.len(),
        report.removed.len(),
        report.unchanged.len(),
        if report.lock_updated {
            "; lock updated"
        } else {
            ""
        }
    );
    cli.emit(&report, message)
}

fn add(cli: &Cli, ws: &mut Workspace, args: &Add) -> Result<()> {
    let (name, mut package) = if args.source.contains("://") {
        enderpin::model::https_url(&args.source)?;
        let kind = args
            .kind
            .context("URL additions require --kind mod|resourcepack|shader|plugin")?;
        let name = args
            .name
            .clone()
            .unwrap_or_else(|| format!("url-{}", &hash_bytes(args.source.as_bytes())[..12]));
        let mut p = Package::modrinth("unused", kind);
        p.modrinth = None;
        p.url = Some(args.source.clone());
        (name, p)
    } else {
        let project = ws.registry.project(&args.source)?;
        let kind = args.kind.unwrap_or(match project.project_type.as_str() {
            "mod"
                if matches!(cli.target, Scope::Server) && ws.manifest.server.loader == "paper" =>
            {
                Kind::Plugin
            }
            "mod" => Kind::Mod,
            "resourcepack" => Kind::Resourcepack,
            "shader" => Kind::Shader,
            other => bail!("unsupported project type {other}"),
        });
        (
            args.name.clone().unwrap_or(project.slug),
            Package::modrinth(project.id, kind),
        )
    };
    identifier(&name)?;
    package.version = args.version_id.clone();
    package.prerelease = args.prerelease;
    package.optional = args.optional.clone();
    package.filename = args.filename.clone();
    package.validate()?;
    let mut manifest = ws.manifest.clone();
    packages_mut(&mut manifest, cli.target).insert(name.clone(), package);
    for side in cli.target.sides() {
        manifest.target_mut(side).disabled.retain(|id| id != &name);
    }
    let options = SyncOptions::default();
    let mut plan = ws.plan(&manifest, &cli.target.sides(), options)?;
    if cli.interactive() && !cli.yes && args.optional.is_empty() {
        let mut optional = BTreeMap::new();
        for side in cli.target.sides() {
            if let Some(target) = plan.targets.get(&side) {
                for root in target.packages.iter().filter(|p| p.roots.contains(&name)) {
                    for dependency in &root.optional {
                        if let Some(id) = &dependency.project {
                            let project = ws.registry.project(id)?;
                            optional.insert(
                                id.clone(),
                                format!(
                                    "{} — {}",
                                    clean(&project.title),
                                    clean(&project.description)
                                ),
                            );
                        }
                    }
                }
            }
        }
        if !optional.is_empty() {
            let values: Vec<_> = optional.into_iter().collect();
            let selected = MultiSelect::new()
                .with_prompt("Optional dependencies (Space: toggle, Enter: continue)")
                .items(values.iter().map(|(_, text)| text))
                .interact_opt()?
                .context("cancelled")?;
            packages_mut(&mut manifest, cli.target)
                .get_mut(&name)
                .context("new package missing")?
                .optional = selected.into_iter().map(|i| values[i].0.clone()).collect();
            plan = ws.plan(&manifest, &cli.target.sides(), options)?;
        }
    }
    apply(cli, ws, manifest, plan, options)
}

fn run(cli: &Cli) -> Result<()> {
    if let Command::Init { minecraft } = &cli.command {
        Workspace::init(&cli.directory, minecraft.clone())?;
        return cli.emit(
            &serde_json::json!({"initialized": true, "minecraft": minecraft}),
            "Created enderpin.toml and enderpin.lock (client + server)",
        );
    }
    let cache = cli
        .cache_dir
        .clone()
        .map(Ok)
        .unwrap_or_else(Cache::default_path)?;
    let mut ws = Workspace::open(&cli.directory, &cache)?;
    match &cli.command {
        Command::Init { .. } => unreachable!(),
        Command::Add(args) => add(cli, &mut ws, args),
        Command::Remove { name } => {
            identifier(name)?;
            let mut manifest = ws.manifest.clone();
            let removed = packages_mut(&mut manifest, cli.target)
                .remove(name)
                .is_some();
            if !matches!(cli.target, Scope::All) && manifest.common.contains_key(name) {
                let target = manifest.target_mut(cli.target.single()?);
                if !target.disabled.contains(name) {
                    target.disabled.push(name.clone());
                }
            } else {
                ensure!(
                    removed,
                    "{name} is not a direct package in the selected scope; use its manifest name and target"
                );
            }
            let options = SyncOptions::default();
            let plan = ws.plan(&manifest, &cli.target.sides(), options)?;
            apply(cli, &mut ws, manifest, plan, options)
        }
        Command::Search {
            query,
            kind,
            offset,
        } => {
            let side = cli.target.single()?;
            let results = ws.registry.search(
                query,
                &ws.manifest.minecraft,
                &ws.manifest.target(side).loader,
                *kind,
                *offset,
            )?;
            if !cli.interactive() {
                let rows = results
                    .hits
                    .iter()
                    .map(|p| {
                        format!(
                            "{}\t{}\t{}",
                            clean(&p.slug),
                            clean(&p.project_id),
                            clean(&p.title)
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                return cli.emit(&results, rows);
            }
            if results.hits.is_empty() {
                println!("No matching projects");
                return Ok(());
            }
            let rows: Vec<_> = results
                .hits
                .iter()
                .map(|p| format!("{} ({})", clean(&p.title), clean(&p.slug)))
                .collect();
            let Some(index) = Select::new()
                .with_prompt(format!(
                    "{} / {} / {} — choose a project (Esc: cancel)",
                    side.name(),
                    clean(&ws.manifest.minecraft),
                    clean(&ws.manifest.target(side).loader)
                ))
                .items(&rows)
                .interact_opt()?
            else {
                return Ok(());
            };
            let hit = &results.hits[index];
            let project = ws.registry.project(&hit.project_id)?;
            let version = ws.registry.versions(&project.id)?.into_iter().find(|v| {
                v.version_type == "release"
                    && v.compatible(
                        &ws.manifest.minecraft,
                        &ws.manifest.target(side).loader,
                        *kind,
                    )
            });
            println!(
                "{}\n{}\nType: {:?}\nTarget: {} / {} / {}\nVersion: {}\n",
                clean(&project.title),
                clean(&project.description),
                kind,
                side.name(),
                clean(&ws.manifest.minecraft),
                clean(&ws.manifest.target(side).loader),
                version
                    .as_ref()
                    .map(|v| clean(&v.version_number))
                    .unwrap_or_else(|| "No compatible release".into())
            );
            if version.is_none()
                || !Confirm::new()
                    .with_prompt("Add this project?")
                    .default(false)
                    .interact()?
            {
                return Ok(());
            }
            add(
                cli,
                &mut ws,
                &Add {
                    source: hit.project_id.clone(),
                    name: None,
                    kind: Some(*kind),
                    version_id: None,
                    prerelease: false,
                    optional: vec![],
                    filename: None,
                },
            )
        }
        Command::List => {
            let mut rows = vec![];
            for side in cli.target.sides() {
                if let Some(target) = ws.lockfile.targets.get(&side) {
                    let stale = target.fingerprint != ws.manifest.fingerprint(side)?;
                    for package in &target.packages {
                        rows.push(
                            serde_json::json!({"target": side, "stale": stale, "package": package}),
                        );
                    }
                    if !cli.json {
                        println!(
                            "{}{}",
                            side.name(),
                            if stale { " (lock is stale)" } else { "" }
                        );
                        for p in &target.packages {
                            println!(
                                "  {}  {}{}",
                                clean(&p.name),
                                clean(p.version_number.as_deref().unwrap_or("URL")),
                                if p.roots.is_empty() {
                                    " (dependency)"
                                } else {
                                    ""
                                }
                            );
                        }
                    }
                }
            }
            if cli.json {
                cli.emit(&rows, "")?;
            }
            Ok(())
        }
        command => {
            let options = match command {
                Command::Sync(s) => SyncOptions {
                    locked: s.locked,
                    offline: s.offline,
                    no_ignore: s.no_ignore,
                    ..Default::default()
                },
                Command::Lock { offline } => SyncOptions {
                    offline: *offline,
                    lock_only: true,
                    ..Default::default()
                },
                Command::Update => SyncOptions {
                    update: true,
                    ..Default::default()
                },
                Command::Restore { offline, no_ignore } => SyncOptions {
                    locked: true,
                    restore: true,
                    offline: *offline,
                    no_ignore: *no_ignore,
                    ..Default::default()
                },
                _ => unreachable!(),
            };
            let manifest = ws.manifest.clone();
            let plan = ws.plan(&manifest, &cli.target.sides(), options)?;
            apply(cli, &mut ws, manifest, plan, options)
        }
    }
}

fn main() {
    let cli = Cli::parse();
    if let Err(error) = run(&cli) {
        if cli.json {
            eprintln!("{}", serde_json::json!({"error": format!("{error:#}")}));
        } else {
            eprintln!("error: {}", clean(&format!("{error:#}")));
        }
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cli_parses_target_and_optional_dependencies() {
        let cli = Cli::try_parse_from([
            "enderpin",
            "add",
            "sodium",
            "--target",
            "all",
            "--optional",
            "a,b",
            "--no-interactive",
        ])
        .expect("valid CLI");
        assert!(matches!(cli.target, Scope::All));
        let Command::Add(args) = cli.command else {
            panic!("expected add");
        };
        assert_eq!(args.optional, vec!["a", "b"]);
        assert_eq!(clean("hello\x1b[2J\u{202e}world"), "hello[2Jworld");
    }
}
