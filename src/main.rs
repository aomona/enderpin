use std::{
    collections::BTreeMap,
    io::{self, IsTerminal},
    path::PathBuf,
};

use anyhow::{Context, Result, bail, ensure};
use clap::{Args, Parser, Subcommand, ValueEnum};
use dialoguer::{Confirm, FuzzySelect, Input, MultiSelect, Select};
use enderpin::{
    Kind, Manifest, Package, Side, SyncOptions, Workspace,
    model::{Lockfile, hash_bytes, identifier},
    storage::Cache,
};
use serde::Serialize;

#[derive(Parser)]
#[command(version, about = "Reproducible Minecraft client and server workspaces")]
struct Cli {
    #[arg(short = 'C', long, global = true)]
    directory: Option<PathBuf>,
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
    /// Set up Minecraft, a loader and mods here using searchable prompts; does not launch the game.
    Quick {
        /// Set up both client and server (default: client only).
        #[arg(long, conflicts_with = "target")]
        server: bool,
        /// Skip the Minecraft version prompt.
        #[arg(long = "version")]
        minecraft_version: Option<String>,
        /// Skip the loader prompt.
        #[arg(long, value_parser = ["vanilla", "fabric"])]
        loader: Option<String>,
        /// Modrinth project IDs or slugs; required dependencies are added automatically.
        #[arg(long, value_delimiter = ',')]
        mods: Vec<String>,
    },
    /// Inspect, approve, revoke or bind this PC's sandbox permissions.
    Permissions {
        #[command(subcommand)]
        action: PermissionCommand,
    },
    /// Sign in using the default Microsoft public-client application or your own.
    Login {
        #[arg(long, default_value = "f8d68570-e721-4aba-9c3e-1052d41e431a")]
        client_id: String,
    },
    /// Delete this CLI's credential from the OS credential store.
    Logout,
    /// Launch a pinned target in the OS sandbox; Ctrl-C stops the server gracefully.
    Launch {
        /// Override the server port for this launch.
        #[arg(long, value_parser = clap::value_parser!(u16).range(1..))]
        port: Option<u16>,
        /// Run a vanilla target without OS sandbox isolation.
        #[arg(long, conflicts_with = "no_network")]
        no_sandbox: bool,
        #[arg(long)]
        offline: bool,
        #[arg(long)]
        no_network: bool,
        /// Explicit acceptance of https://aka.ms/MinecraftEULA for this local server.
        #[arg(long)]
        accept_eula: bool,
        #[arg(long, default_value_t = 2048)]
        memory: u32,
        /// Launch the official Minecraft demo without an account (no multiplayer).
        #[arg(long)]
        demo: bool,
        /// Join this server on launch (Minecraft versions supporting Quick Play).
        #[arg(long)]
        connect: Option<String>,
    },
    /// Pin and download Minecraft, Fabric and a platform-specific Temurin JDK.
    Prepare {
        #[arg(long)]
        locked: bool,
        #[arg(long)]
        offline: bool,
        #[arg(long)]
        update: bool,
    },
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

#[derive(Subcommand)]
enum PermissionCommand {
    /// Show the complete plan and its approval fingerprint. Run sync first.
    Show,
    /// Approve exactly the inspected plan; --yes never approves permissions.
    Approve { fingerprint: String },
    /// Forget approval; already running games must be stopped first.
    Revoke,
    /// Bind a logical folder slot to an existing local directory; clears approval.
    Bind { name: String, path: PathBuf },
    /// Remove a local folder binding and its approval.
    Unbind { name: String },
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
    fn workspace_directory(&self) -> &std::path::Path {
        self.directory
            .as_deref()
            .unwrap_or_else(|| std::path::Path::new("."))
    }
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

fn permission_text(plan: &enderpin::sandbox::permissions::Plan) -> Result<String> {
    Ok(serde_json::to_string_pretty(plan)?
        .lines()
        .map(clean)
        .collect::<Vec<_>>()
        .join("\n"))
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

fn choose(prompt: &str, items: &[String], default: usize) -> Result<usize> {
    FuzzySelect::new()
        .with_prompt(format!(
            "{prompt} (type to search, Enter: select, Esc: cancel)"
        ))
        .items(items)
        .default(default)
        .max_length(12)
        .interact_opt()?
        .context("setup cancelled")
}

fn setup_mod(
    registry: &mut enderpin::registry::Modrinth,
    id: &str,
    minecraft: &str,
    loader: &str,
    scope: Scope,
) -> Result<(String, Package)> {
    let project = registry.project(id)?;
    ensure!(
        project.project_type == "mod",
        "{} is not a mod",
        clean(&project.title)
    );
    let version = registry
        .versions(&project.id)?
        .into_iter()
        .find(|version| {
            version.version_type == "release"
                && version.compatible(minecraft, loader, Kind::Mod)
                && scope
                    .sides()
                    .iter()
                    .any(|&side| version.default_enabled(&project, side, Kind::Mod))
        })
        .context("no compatible released mod version for the selected target")?;
    let mut package = Package::modrinth(project.id, Kind::Mod);
    package.version = Some(version.id);
    package.validate()?;
    Ok((project.slug, package))
}

fn choose_mods(
    registry: &mut enderpin::registry::Modrinth,
    minecraft: &str,
    loader: &str,
    scope: Scope,
    selected: &mut BTreeMap<String, Package>,
) -> Result<()> {
    let search = || -> Result<String> {
        Ok(Input::<String>::new()
            .with_prompt("Search Modrinth mods (empty: finish selection)")
            .allow_empty(true)
            .validate_with(|query: &String| -> Result<(), &str> {
                if query.len() <= 400 {
                    Ok(())
                } else {
                    Err("Use at most 400 bytes")
                }
            })
            .interact_text()?)
    };
    let mut query = search()?;
    let mut offset = 0;
    while !query.is_empty() {
        eprintln!("Searching Modrinth...");
        let results = registry.search(&query, minecraft, loader, Kind::Mod, offset)?;
        let next = offset + (results.hits.len() as u64);
        let more = !results.hits.is_empty() && next < results.total_hits;
        loop {
            let mut rows: Vec<_> = results
                .hits
                .iter()
                .map(|hit| {
                    format!(
                        "[{}] {} ({})",
                        if selected.contains_key(&hit.slug) {
                            "x"
                        } else {
                            " "
                        },
                        clean(&hit.title),
                        clean(&hit.slug)
                    )
                })
                .collect();
            let count = rows.len();
            rows.extend([
                "Search again".into(),
                format!("Finish selection ({} mods)", selected.len()),
            ]);
            if more {
                rows.push("Next page".into());
            }
            let index = choose("Mods — select to add/remove", &rows, 0)?;
            if index < count {
                let hit = &results.hits[index];
                if selected.remove(&hit.slug).is_none() {
                    match setup_mod(registry, &hit.project_id, minecraft, loader, scope) {
                        Ok((name, package)) => {
                            selected.insert(name, package);
                        }
                        Err(error) => {
                            eprintln!("{}: {}", clean(&hit.title), clean(&format!("{error:#}")))
                        }
                    }
                }
            } else if index == count {
                query = search()?;
                offset = 0;
                break;
            } else if index == count + 1 {
                return Ok(());
            } else {
                offset = next;
                break;
            }
        }
    }
    Ok(())
}

fn quick_setup(
    cli: &Cli,
    server: bool,
    version: Option<&str>,
    loader: Option<&str>,
    mods: &[String],
) -> Result<()> {
    ensure!(
        matches!(cli.target, Scope::Client),
        "quick uses --server for both targets; do not use --target"
    );
    ensure!(!cli.json, "quick setup does not support --json");
    ensure!(
        cli.interactive() || (version.is_some() && loader.is_some()),
        "quick needs a terminal for setup; use --version VERSION --loader vanilla|fabric in non-interactive mode"
    );
    let root = cli.workspace_directory();
    ensure!(
        enderpin::storage::read_optional(root, "enderpin.toml")?.is_none()
            && enderpin::storage::read_optional(root, "enderpin.lock")?.is_none(),
        "workspace already exists; use add, prepare or launch, or choose a new directory with -C"
    );
    let minecraft = if let Some(version) = version {
        identifier(version)?;
        version.to_owned()
    } else {
        eprintln!("Loading Minecraft versions...");
        let versions = enderpin::runtime::minecraft_versions()?;
        ensure!(!versions.is_empty(), "no Minecraft versions available");
        let rows: Vec<_> = versions
            .iter()
            .map(|v| format!("{} ({})", clean(&v.id), clean(&v.kind)))
            .collect();
        let default = versions
            .iter()
            .position(|v| v.kind == "release")
            .unwrap_or(0);
        versions[choose("Minecraft version", &rows, default)?]
            .id
            .clone()
    };
    let loader = if let Some(loader) = loader {
        loader.to_owned()
    } else {
        let loaders = ["Vanilla (no mods)".to_owned(), "Fabric".to_owned()];
        ["vanilla", "fabric"][choose("Mod loader", &loaders, 0)?].to_owned()
    };
    ensure!(
        ["vanilla", "fabric"].contains(&loader.as_str()),
        "unsupported setup loader"
    );
    ensure!(
        mods.is_empty() || loader != "vanilla",
        "vanilla cannot load mods; select Fabric"
    );
    let scope = if server { Scope::All } else { Scope::Client };
    let mut registry = enderpin::registry::Modrinth::default();
    let mut selected = BTreeMap::new();
    for id in mods {
        let (name, package) = setup_mod(&mut registry, id, &minecraft, &loader, scope)?;
        selected.insert(name, package);
    }
    if cli.interactive() && loader != "vanilla" {
        choose_mods(&mut registry, &minecraft, &loader, scope, &mut selected)?;
    }
    eprintln!(
        "\nMinecraft {minecraft} / {loader} / {}\nMods: {}\nDirectory: {}",
        if server { "client + server" } else { "client" },
        if selected.is_empty() {
            "none".into()
        } else {
            selected.keys().cloned().collect::<Vec<_>>().join(", ")
        },
        root.display()
    );
    if cli.interactive() {
        ensure!(
            Confirm::new()
                .with_prompt("Create workspace and download files?")
                .default(true)
                .interact()?,
            "setup cancelled"
        );
    }
    let mut manifest = Workspace::quick_manifest(minecraft)?;
    for side in scope.sides() {
        manifest.target_mut(side).loader = loader.clone();
    }
    *packages_mut(&mut manifest, scope) = selected;
    Workspace::init_manifest(root, manifest, &Side::ALL)?;
    let cache = cli
        .cache_dir
        .clone()
        .map(Ok)
        .unwrap_or_else(Cache::default_path)?;
    let mut ws = Workspace::open(root, &cache)?;
    ws.registry = registry;
    ws.prepare_runtime(
        &scope.sides(),
        SyncOptions::default(),
        false,
        |side, message| {
            eprintln!("{}: {}", side.name(), clean(message));
        },
    )?;
    eprintln!("Setup complete. Start the client with enderpin launch.");
    if server {
        eprintln!(
            "Start the server with enderpin launch --target server (requires EULA acceptance)."
        );
    }
    Ok(())
}

fn run(cli: &Cli) -> Result<()> {
    if let Command::Quick {
        server,
        minecraft_version,
        loader,
        mods,
    } = &cli.command
    {
        return quick_setup(
            cli,
            *server,
            minecraft_version.as_deref(),
            loader.as_deref(),
            mods,
        );
    }
    if let Command::Login { client_id } = &cli.command {
        ensure!(
            !cli.json,
            "login displays an interactive authorization code; --json is not supported"
        );
        let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let signal = cancelled.clone();
        ctrlc::set_handler(move || {
            signal.store(true, std::sync::atomic::Ordering::Relaxed);
        })?;
        let session = enderpin::auth::login(
            client_id,
            |prompt| {
                eprintln!(
                    "Open {} and enter {} (expires in {} seconds).",
                    prompt.verification_uri, prompt.user_code, prompt.expires_in
                );
                Ok(())
            },
            || cancelled.load(std::sync::atomic::Ordering::Relaxed),
        )?;
        return cli.emit(
            &serde_json::json!({"name": session.name, "uuid": session.uuid}),
            format!(
                "Signed in as {}. Credential saved in the OS credential store.",
                session.name
            ),
        );
    }
    if matches!(cli.command, Command::Logout) {
        enderpin::auth::logout()?;
        return cli.emit(&serde_json::json!({"signed_out": true}), "Enderpin credential removed and broker access revoked. Existing server connections are not disconnected.");
    }
    if let Command::Init { minecraft } = &cli.command {
        Workspace::init(cli.workspace_directory(), minecraft.clone())?;
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
    let mut ws = Workspace::open(cli.workspace_directory(), &cache)?;
    match &cli.command {
        Command::Permissions { action } => {
            use enderpin::sandbox::permissions::{self, Local};
            let side = cli.target.single()?;
            let mut local = Local::load(&ws, side)?;
            match action {
                PermissionCommand::Show => {
                    let plan = permissions::plan(&ws, side)?;
                    let fingerprint = plan.fingerprint()?;
                    return cli.emit(&serde_json::json!({"fingerprint": fingerprint, "approved": local.approved.as_deref() == Some(&fingerprint), "plan": plan}), format!("{}\nFingerprint: {fingerprint}\nApprove with: enderpin --target {} permissions approve {fingerprint}", permission_text(&plan)?, side.name()));
                }
                PermissionCommand::Approve { fingerprint } => {
                    let plan = permissions::plan(&ws, side)?;
                    ensure!(
                        fingerprint == &plan.fingerprint()?,
                        "permission plan changed; inspect permissions show again"
                    );
                    local.approved = Some(fingerprint.clone());
                }
                PermissionCommand::Revoke => local.approved = None,
                PermissionCommand::Bind { name, path } => {
                    identifier(name)?;
                    let path = path.canonicalize().context("folder must already exist")?;
                    ensure!(
                        path.is_dir() && !ws.root.starts_with(&path) && !path.starts_with(&ws.root),
                        "binding must be an external directory, separate from the workspace"
                    );
                    enderpin::sandbox::validate_data_tree(&path)?;
                    local.bindings.insert(name.clone(), path);
                    local.approved = None;
                }
                PermissionCommand::Unbind { name } => {
                    local.bindings.remove(name);
                    local.approved = None;
                }
            }
            local.save(&ws, side)?;
            cli.emit(
                &serde_json::json!({"updated": true, "approved": local.approved.is_some()}),
                "Local permissions updated",
            )
        }
        Command::Launch {
            port,
            no_sandbox,
            offline,
            no_network,
            accept_eula,
            memory,
            demo,
            connect,
        } => {
            ensure!(
                !cli.json,
                "launch streams game output; --json is not supported"
            );
            let side = cli.target.single()?;
            ensure!(
                port.is_none() || side == Side::Server,
                "--port is only for servers"
            );
            ensure!(!*demo || side == Side::Client, "--demo is only for clients");
            ensure!(
                !*accept_eula || side == Side::Server,
                "--accept-eula is only for servers"
            );
            ws.sync(
                ws.manifest.clone(),
                &[side],
                SyncOptions {
                    locked: true,
                    offline: *offline,
                    ..Default::default()
                },
                |_| {},
            )?;
            let plan = enderpin::sandbox::permissions::plan(&ws, side)?;
            let mut local = enderpin::sandbox::permissions::Local::load(&ws, side)?;
            let fingerprint = plan.fingerprint()?;
            if !no_sandbox && local.approved.as_deref() != Some(&fingerprint) {
                ensure!(
                    cli.interactive(),
                    "sandbox permissions need approval; run permissions show, then permissions approve <fingerprint>"
                );
                eprintln!("{}", permission_text(&plan)?);
                ensure!(
                    Confirm::new()
                        .with_prompt(
                            "Approve these permissions for the entire Minecraft process on this PC?"
                        )
                        .default(false)
                        .interact()?,
                    "permissions were not approved"
                );
                local.approved = Some(fingerprint);
                local.save(&ws, side)?;
            }
            let stopping = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let signal = stopping.clone();
            ctrlc::set_handler(move || {
                signal.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            })?;
            let session = if side == Side::Client && !*demo && plan.effective.account_authentication
            {
                Some(enderpin::auth::session()?)
            } else {
                None
            };
            let mut game = enderpin::launch::start(
                ws,
                side,
                enderpin::launch::LaunchOptions {
                    sandbox: !no_sandbox,
                    offline: *offline,
                    network: !no_network,
                    accept_eula: *accept_eula,
                    memory_mib: *memory,
                    port: *port,
                    connect: connect.clone(),
                },
                session.as_ref(),
                *demo,
                |message| eprintln!("{}: {}", side.name(), clean(message)),
            )?;
            eprintln!(
                "{} {} started (PID {}). Press Ctrl-C to stop.",
                if *no_sandbox {
                    "Unconfined"
                } else {
                    "Sandboxed"
                },
                side.name(),
                game.id()
            );
            let (sender, receiver) = std::sync::mpsc::channel();
            if side == Side::Server {
                std::thread::spawn(move || {
                    use std::io::BufRead;
                    for line in io::stdin().lock().lines() {
                        let Ok(line) = line else {
                            break;
                        };
                        if sender.send(line).is_err() {
                            return;
                        }
                    }
                    let _ = sender.send("stop".into());
                });
            }
            let mut stop_requested = false;
            loop {
                if let Some(status) = game.try_wait()? {
                    ensure!(
                        status.success() || (side == Side::Client && stop_requested),
                        "{} exited with {status}",
                        side.name()
                    );
                    return Ok(());
                }
                if stopping.load(std::sync::atomic::Ordering::Relaxed) > 0 && !stop_requested {
                    game.stop(side)?;
                    stop_requested = true;
                    eprintln!("Waiting for shutdown (Ctrl-C again forces termination).");
                }
                while let Ok(line) = receiver.try_recv() {
                    game.console(&line)?;
                    if line.trim() == "stop" {
                        stop_requested = true;
                    }
                }
                if stopping.load(std::sync::atomic::Ordering::Relaxed) > 1 {
                    game.kill()?;
                    bail!("game was forcibly terminated after shutdown request");
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
        Command::Prepare {
            locked,
            offline,
            update,
        } => {
            let prepared = ws.prepare_runtime(
                &cli.target.sides(),
                SyncOptions {
                    locked: *locked,
                    offline: *offline,
                    ..Default::default()
                },
                *update,
                |side, message| {
                    if !cli.json {
                        eprintln!("{}: {}", side.name(), clean(message));
                    }
                },
            )?;
            let summary: Vec<_> = prepared.iter().map(|(side, runtime)| serde_json::json!({"target": side, "java": runtime.java, "classpath_entries": runtime.classpath.len(), "assets": runtime.assets})).collect();
            cli.emit(&summary, "Runtime prepared and pinned in enderpin.lock")
        }
        Command::Quick { .. } | Command::Init { .. } | Command::Login { .. } | Command::Logout => {
            unreachable!()
        }
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
    fn setup_options_require_explicit_noninteractive_choices() -> Result<()> {
        let root = tempfile::tempdir()?;
        let directory = root.path().to_str().context("non-UTF8 temporary path")?;
        for flags in [
            vec![],
            vec!["--version", "26.2"],
            vec!["--loader", "vanilla"],
            vec!["--version", "../bad", "--loader", "vanilla"],
            vec![
                "--version",
                "26.2",
                "--loader",
                "vanilla",
                "--mods",
                "sodium",
            ],
        ] {
            let cli = Cli::try_parse_from(
                [
                    vec!["enderpin", "-C", directory, "--no-interactive", "quick"],
                    flags,
                ]
                .concat(),
            )?;
            assert!(run(&cli).is_err());
            assert!(!root.path().join("enderpin.toml").exists());
        }
        let cli = Cli::try_parse_from([
            "enderpin",
            "quick",
            "--server",
            "--version",
            "26.2",
            "--loader",
            "fabric",
            "--mods",
            "lithium,sodium",
        ])?;
        assert!(matches!(cli.command, Command::Quick {server: true, mods, ..} if mods.len() == 2));
        assert!(Cli::try_parse_from(["enderpin", "quick", "--no-sandbox"]).is_err());
        Workspace::init_quick(root.path(), "26.2".into())?;
        let original = std::fs::read(root.path().join("enderpin.toml"))?;
        let cli = Cli::try_parse_from([
            "enderpin",
            "-C",
            directory,
            "--no-interactive",
            "quick",
            "--version",
            "26.2",
            "--loader",
            "vanilla",
        ])?;
        assert!(run(&cli).is_err());
        assert_eq!(std::fs::read(root.path().join("enderpin.toml"))?, original);
        Ok(())
    }
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
