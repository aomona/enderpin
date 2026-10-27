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
#[command(
    version,
    about = "Reproducible Minecraft client and server workspaces",
    help_template = "{about-with-newline}\n{usage-heading} {usage}\n\nGetting started:\n  init         Create a workspace and download Minecraft, Java and packages\n\nEveryday:\n  add          Add a package and synchronize\n  remove       Remove a package and synchronize\n  search       Find packages on Modrinth\n  list         Show locked packages\n  sync         Prepare runtimes, packages and shared files without updating pins\n  run          Run a locked client or server\n  update       Update runtimes and Modrinth packages\n\nSettings & authentication:\n  permissions  Review, edit and approve game permissions\n  login        Sign in with Microsoft\n  logout       Remove this CLI's saved credential\n\nAutomation & recovery:\n  wrapper      Generate repository launchers pinned to an Enderpin release\n  lock         Resolve packages without installing files\n  restore      Restore managed files, discarding local changes\n  help         Show command help\n\nOptions:\n{options}\n\nExamples:\n  enderpin init my-world --loader fabric\n  cd my-world\n  enderpin add sodium\n  enderpin login\n  enderpin run\n\nUse enderpin <COMMAND> --help for options and examples.\n"
)]
struct Cli {
    /// Workspace directory (default: current directory).
    #[arg(short = 'C', long, global = true, value_name = "DIRECTORY")]
    directory: Option<PathBuf>,
    /// Shared download cache directory.
    #[arg(long, global = true, value_name = "DIRECTORY")]
    cache_dir: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

fn single_target(name: &str) -> Result<String, String> {
    enderpin::model::target_name(name).map_err(|e| e.to_string())?;
    Ok(name.into())
}
fn target_selection(name: &str) -> Result<String, String> {
    if name == "all" {
        Ok(name.into())
    } else {
        single_target(name)
    }
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
}

#[derive(Args)]
struct Selection {
    /// Target name; all selects every declared target.
    #[arg(long, default_value = "client", value_parser = target_selection)]
    target: String,
    /// Emit machine-readable JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
struct SingleSelection {
    /// Operate on one named target.
    #[arg(long, global = true, value_parser = single_target, default_value = "client")]
    target: String,
    /// Emit machine-readable JSON.
    #[arg(long, global = true)]
    json: bool,
}

#[derive(Args)]
struct Prompts {
    /// Disable prompts; supply required choices as options.
    #[arg(long)]
    no_interactive: bool,
    /// Accept setup/default dependency choices; never approve game permissions.
    #[arg(short, long)]
    yes: bool,
}

#[derive(Args)]
#[command(
    after_help = "Examples:\n  enderpin init my-world\n  enderpin init my-server --target server --minecraft 1.21.1 --loader paper --no-interactive\n  enderpin init --minecraft 1.21.1 --loader fabric --loader-version 0.16.10 --mods sodium --no-interactive"
)]
struct Init {
    /// New workspace directory (default: current directory).
    #[arg(value_name = "DIRECTORY")]
    path: Option<PathBuf>,
    #[arg(long, value_parser = target_selection, default_value = "client")]
    target: String,
    /// Execution kind when creating a named target.
    #[arg(long = "type", value_enum)]
    kind: Option<enderpin::model::TargetType>,
    /// Minecraft version; prompts when omitted.
    #[arg(long, value_name = "VERSION")]
    minecraft: Option<String>,
    /// Runtime type; Paper and Velocity require --target server.
    #[arg(long, value_parser = ["vanilla", "fabric", "paper", "velocity"])]
    loader: Option<String>,
    /// Exact loader version/build (required for Velocity); otherwise resolve once and pin.
    #[arg(
        long,
        value_name = "VERSION",
        requires = "loader",
        required_if_eq("loader", "velocity")
    )]
    loader_version: Option<String>,
    /// Fabric Modrinth IDs/slugs, separated by commas; includes required dependencies.
    #[arg(long, value_delimiter = ',')]
    mods: Vec<String>,
    #[command(flatten)]
    prompts: Prompts,
}

#[derive(Subcommand)]
enum Command {
    /// Create or initialize targets and native settings; does not start a game session or approve it.
    Init(Init),
    /// Generate repository launchers pinned to a verified GitHub release.
    Wrapper {
        /// Enderpin release version (default: latest stable, resolved only when generating).
        #[arg(long, value_name = "VERSION")]
        version: Option<String>,
        /// Replace all three existing wrapper files.
        #[arg(long)]
        force: bool,
    },
    /// Edit permissions, inspect a plan or approve its exact fingerprint.
    #[command(
        after_help = "Examples:\n  enderpin permissions\n  enderpin permissions show --target server --json\n  enderpin permissions approve --target server <FINGERPRINT>"
    )]
    Permissions {
        #[command(flatten)]
        selection: SingleSelection,
        /// Disable interactive editing and approval prompts.
        #[arg(long, global = true)]
        no_interactive: bool,
        #[command(subcommand)]
        action: Option<PermissionCommand>,
    },
    /// Sign in using Microsoft's default public-client application or your own.
    Login {
        #[arg(long, default_value = "f8d68570-e721-4aba-9c3e-1052d41e431a")]
        client_id: String,
    },
    /// Remove this CLI's credential from the OS credential store.
    Logout,
    /// Run a pinned target in the OS sandbox, downloading missing locked files as needed.
    #[command(
        after_help = "Examples:\n  enderpin run\n  enderpin run --target server --accept-eula --memory 1024\n  enderpin run --offline --demo\n  enderpin run --target server --jvm-arg -XX:+UseZGC\n\nRun never updates pinned versions. Change configuration with sync, or versions with update.\nPermission approval is always explicit; --offline controls downloads, --no-network controls the game."
    )]
    Run {
        #[arg(long, value_parser = single_target, default_value = "client")]
        target: String,
        /// Disable interactive permission approval.
        #[arg(long)]
        no_interactive: bool,
        /// Server only: ignore terminal input; stop gracefully on SIGTERM or Ctrl-C.
        #[arg(long)]
        service: bool,
        /// Server only: override the port for this run (Velocity uses velocity.toml).
        #[arg(long, value_parser = clap::value_parser!(u16).range(1..))]
        port: Option<u16>,
        /// Vanilla only: run without OS sandbox isolation.
        #[arg(long, conflicts_with = "no_network")]
        no_sandbox: bool,
        /// Use only locked files already available in the local cache.
        #[arg(long)]
        offline: bool,
        /// Deny game network access for this run.
        #[arg(long)]
        no_network: bool,
        /// Server only: explicitly accept https://aka.ms/MinecraftEULA.
        #[arg(long)]
        accept_eula: bool,
        /// Maximum Java heap in MiB.
        #[arg(long, value_parser = clap::value_parser!(u32).range(256..=1_048_576), value_name = "MIB")]
        memory: Option<u32>,
        /// Client only: run the official demo without an account (no multiplayer).
        #[arg(long, conflicts_with = "connect")]
        demo: bool,
        /// Client only: join a server (requires authentication and Quick Play support).
        #[arg(long, conflicts_with = "no_network")]
        connect: Option<String>,
        /// Extra JVM option for this run only (repeatable; -XX: options and -D properties).
        #[arg(long = "jvm-arg", value_name = "ARG", allow_hyphen_values = true, value_parser = jvm_arg)]
        jvm_args: Vec<String>,
    },
    /// Add a Modrinth project or a public HTTPS file URL and synchronize packages.
    #[command(
        after_help = "Examples:\n  enderpin add sodium\n  enderpin add lithium --target all\n  enderpin add https://example.com/pack.zip --kind resourcepack --name my-pack"
    )]
    Add {
        #[command(flatten)]
        selection: Selection,
        #[command(flatten)]
        prompts: Prompts,
        #[command(flatten)]
        package: PackageArgs,
    },
    /// Remove a direct package; retain dependencies still needed by other packages.
    Remove {
        name: String,
        #[command(flatten)]
        selection: Selection,
    },
    /// Search Modrinth; choose and install a result in a terminal.
    Search {
        #[arg(default_value = "")]
        query: String,
        #[command(flatten)]
        selection: SingleSelection,
        #[command(flatten)]
        prompts: Prompts,
        #[arg(long, value_enum, default_value = "mod")]
        kind: Kind,
        #[arg(long, default_value_t = 0)]
        offset: u64,
    },
    /// Prepare Minecraft, Java, packages and shared files, preserving pinned versions.
    #[command(
        after_help = "Examples:\n  enderpin sync --target all\n  enderpin sync --target server --locked\n  enderpin sync --locked --offline\n\n--locked requires matching package and runtime locks. --offline also requires cached downloads.\nUse update to select newer versions; sync only resolves new or changed configuration."
    )]
    Sync {
        #[command(flatten)]
        selection: Selection,
        #[command(flatten)]
        options: SyncArgs,
    },
    /// Resolve and save package locks without installing files or preparing runtimes.
    Lock {
        #[command(flatten)]
        selection: Selection,
        #[arg(long)]
        offline: bool,
    },
    /// Update runtimes and Modrinth packages; explicit loader versions and URL pins stay fixed.
    Update {
        #[command(flatten)]
        selection: Selection,
    },
    /// Discard local changes to managed files and restore their locked/shared contents.
    Restore {
        #[command(flatten)]
        selection: Selection,
        #[arg(long)]
        offline: bool,
        /// Ignore local package exclusions in .enderpinignore.
        #[arg(long)]
        no_ignore: bool,
    },
    /// Show locked packages and whether each target's package lock matches its configuration.
    List {
        #[command(flatten)]
        selection: Selection,
    },
}

#[derive(Subcommand)]
enum PermissionCommand {
    /// Show a readable summary; --json includes the complete plan and fingerprint.
    Show,
    /// Review and confirm in a terminal, or approve an exact fingerprint in automation.
    Approve { fingerprint: Option<String> },
    /// Forget approval; already running games must be stopped first.
    Revoke,
    /// Allow an existing external folder; read-only unless --write is given. Clears approval.
    AllowFolder {
        path: PathBuf,
        #[arg(long)]
        write: bool,
    },
    /// Remove an external folder and its approval (the folder itself is kept).
    RemoveFolder { path: PathBuf },
}

#[derive(Args)]
struct PackageArgs {
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
struct SyncArgs {
    /// Require matching package and runtime locks; do not resolve configuration changes.
    #[arg(long)]
    locked: bool,
    /// Use existing locks and cached downloads only.
    #[arg(long)]
    offline: bool,
    /// Ignore local package exclusions in .enderpinignore.
    #[arg(long)]
    no_ignore: bool,
}

impl Cli {
    fn target(&self) -> &str {
        match &self.command {
            Command::Init(args) => &args.target,
            Command::Run { target, .. } => target,
            Command::Permissions { selection, .. } | Command::Search { selection, .. } => {
                &selection.target
            }
            Command::Add { selection, .. }
            | Command::Remove { selection, .. }
            | Command::Sync { selection, .. }
            | Command::Lock { selection, .. }
            | Command::Update { selection }
            | Command::Restore { selection, .. }
            | Command::List { selection } => &selection.target,
            _ => "client",
        }
    }
    fn single_target(&self) -> Result<&str> {
        ensure!(
            self.target() != "all",
            "this command requires one named target"
        );
        Ok(self.target())
    }
    fn targets(&self, manifest: &Manifest) -> Result<Vec<String>> {
        if self.target() == "all" {
            Ok(manifest.targets.keys().cloned().collect())
        } else {
            manifest.target(self.target())?;
            Ok(vec![self.target().into()])
        }
    }
    fn json(&self) -> bool {
        match &self.command {
            Command::Permissions { selection, .. } | Command::Search { selection, .. } => {
                selection.json
            }
            Command::Add { selection, .. }
            | Command::Remove { selection, .. }
            | Command::Sync { selection, .. }
            | Command::Lock { selection, .. }
            | Command::Update { selection }
            | Command::Restore { selection, .. }
            | Command::List { selection } => selection.json,
            _ => false,
        }
    }
    fn prompts(&self) -> Option<&Prompts> {
        match &self.command {
            Command::Init(args) => Some(&args.prompts),
            Command::Add { prompts, .. } | Command::Search { prompts, .. } => Some(prompts),
            _ => None,
        }
    }
    fn workspace_directory(&self) -> &std::path::Path {
        self.directory
            .as_deref()
            .unwrap_or_else(|| std::path::Path::new("."))
    }
    fn interactive(&self) -> bool {
        let disabled = self.prompts().is_some_and(|p| p.no_interactive)
            || matches!(
                &self.command,
                Command::Run {
                    no_interactive: true,
                    ..
                } | Command::Permissions {
                    no_interactive: true,
                    ..
                }
            );
        !disabled
            && !self.json()
            && io::stdin().is_terminal()
            && io::stdout().is_terminal()
            && io::stderr().is_terminal()
    }
    fn yes(&self) -> bool {
        self.prompts().is_some_and(|p| p.yes)
    }
    fn emit(&self, value: &impl Serialize, message: impl AsRef<str>) -> Result<()> {
        if self.json() {
            println!("{}", serde_json::to_string_pretty(value)?);
        } else {
            println!("{}", message.as_ref());
        }
        Ok(())
    }
}

fn jvm_arg(value: &str) -> Result<String> {
    enderpin::model::jvm_argument(value)?;
    Ok(value.to_owned())
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

fn permission_text(plan: &enderpin::sandbox::permissions::Plan) -> String {
    let settings = &plan.effective;
    let access = |allowed| if allowed { "allowed" } else { "denied" };
    let mut lines = vec![
        format!(
            "{} — Minecraft {} / {}{}",
            plan.target,
            clean(&plan.minecraft),
            clean(&plan.loader),
            plan.loader_version
                .as_ref()
                .map(|v| format!(" {}", clean(v)))
                .unwrap_or_default()
        ),
        format!(
            "Workspace: {}",
            clean(&plan.workspace.display().to_string())
        ),
        format!(
            "Network (internet, LAN, listening): {}",
            access(settings.network)
        ),
        format!(
            "Game folder: {}",
            if settings.files.game.writable() {
                "read/write"
            } else {
                "read-only"
            }
        ),
    ];
    if !plan.shared_files.is_empty() {
        lines.push("Shared files (approval includes their installed contents):".into());
        lines.extend(
            plan.shared_files
                .keys()
                .map(|path| format!("  {}", clean(path))),
        );
    }
    if settings.files.game.writable() && !settings.files.read_only.is_empty() {
        lines.push(format!(
            "Read-only game folders: {}",
            settings
                .files
                .read_only
                .iter()
                .map(|d| d.name())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if plan.side == Side::Client {
        lines.push(format!(
            "Account authentication via host: {} (independent of game network)",
            access(settings.account_authentication)
        ));
        lines.push(format!(
            "Audio: {}; narrator: {}; skin cache: {}",
            access(settings.audio),
            access(settings.narrator),
            access(settings.skin_cache)
        ));
        lines.push(format!(
            "Desktop integration: {}; graphics cache: {}",
            access(settings.desktop_integration),
            access(settings.graphics_cache)
        ));
        lines.push(format!(
            "Import files with host confirmation: {}",
            access(settings.import_files)
        ));
        lines.push(format!(
            "Open web links: {}; open folders/screenshots: {}",
            access(settings.open_links),
            access(settings.open_folders)
        ));
        if plan.platform == "macos" {
            lines.push(format!(
                "Microphone: {}; clipboard: {}",
                access(settings.microphone == Some(true)),
                access(settings.clipboard_allowed())
            ));
        }
    }
    if !plan.jvm_args.is_empty() {
        lines.push(format!(
            "JVM arguments from enderpin.toml: {}",
            plan.jvm_args
                .iter()
                .map(|arg| clean(arg))
                .collect::<Vec<_>>()
                .join(" ")
        ));
    }
    lines.push(format!("External folders: {}", plan.folders.len()));
    for folder in &plan.folders {
        lines.push(format!(
            "  {} — {}",
            clean(&folder.path.display().to_string()),
            if folder.write {
                "read/write"
            } else {
                "read-only"
            }
        ));
    }
    lines.push(format!(
        "Packages: {} (adding or updating one requires approval again)",
        plan.packages.len()
    ));
    for package in &plan.packages {
        lines.push(format!(
            "  {}{} [file {}]",
            clean(&package.name),
            package
                .version
                .as_ref()
                .map(|v| format!(" {}", clean(v)))
                .unwrap_or_default(),
            &package.sha512[..12]
        ));
    }
    lines.extend(plan.limitations.iter().map(|note| clean(note)));
    lines.join("\n")
}

fn approve_permissions(
    cli: &Cli,
    ws: &Workspace,
    side: &str,
    fingerprint: Option<&str>,
) -> Result<()> {
    use enderpin::sandbox::permissions::{self, Local};
    let plan = permissions::plan(ws, side)?;
    let current = plan.fingerprint()?;
    if let Some(fingerprint) = fingerprint {
        ensure!(
            fingerprint == current,
            "permission plan changed; inspect permissions show again"
        );
    } else {
        ensure!(
            cli.interactive(),
            "permission approval needs a terminal; for automation use permissions show --json, then permissions approve <fingerprint>; --yes does not approve permissions"
        );
        eprintln!("{}", permission_text(&plan));
        ensure!(
            Confirm::new()
                .with_prompt("Approve this game and these packages on this PC?")
                .default(false)
                .interact()?,
            "permissions were not approved"
        );
    }
    // Recheck after the prompt, in case an installed file changed while it was displayed.
    ensure!(
        permissions::plan(ws, side)?.fingerprint()? == current,
        "permission plan changed; review it again"
    );
    let mut local = Local::load(ws, side)?;
    local.approved = Some(current);
    local.save(ws, side)
}

fn edit_switches(switches: &mut [(&str, &mut bool)]) -> Result<()> {
    let selected = MultiSelect::new()
        .with_prompt("Allowed access — Space to toggle, Enter to keep, Esc to cancel")
        .items(switches.iter().map(|(label, _)| *label).collect::<Vec<_>>())
        .defaults(
            &switches
                .iter()
                .map(|(_, value)| **value)
                .collect::<Vec<_>>(),
        )
        .interact_opt()?;
    if let Some(selected) = selected {
        for (index, (_, value)) in switches.iter_mut().enumerate() {
            **value = selected.contains(&index);
        }
    }
    Ok(())
}

fn allow_folder(
    ws: &Workspace,
    local: &mut enderpin::sandbox::permissions::Local,
    path: &std::path::Path,
    write: bool,
) -> Result<()> {
    use enderpin::sandbox::permissions::{FolderGrant, validate_folders};
    let path = path.canonicalize().context("folder must already exist")?;
    let mut folders = local.folders.clone();
    folders.retain(|folder| folder.path != path);
    folders.push(FolderGrant { path, write });
    folders.sort_by(|a, b| a.path.cmp(&b.path));
    validate_folders(ws, &folders)?;
    local.folders = folders;
    local.approved = None;
    Ok(())
}

fn edit_permissions(cli: &Cli, ws: &mut Workspace, side: &str) -> Result<()> {
    use enderpin::sandbox::permissions::{Clipboard, FileAccess, GameDirectory, Local};
    ensure!(
        cli.interactive(),
        "permissions editor needs a terminal; use permissions show --json to inspect or edit enderpin.toml"
    );
    let mut manifest = ws.manifest.clone();
    let kind = manifest.target(side)?.side();
    let settings = manifest
        .target_mut(side)?
        .permissions
        .get_or_insert_with(Default::default);
    let mut local = Local::load(ws, side)?;
    eprintln!(
        "Editing {} permissions. All mods share these permissions. Cancel from the main menu discards unsaved changes.",
        side
    );
    loop {
        let mut items = vec![
            "Save and review",
            "Game permissions",
            "Read-only game folders",
            "External folders",
        ];
        if kind == Side::Client {
            items.push("Advanced desktop permissions");
            if settings.validate_for(std::env::consts::OS, true).is_err() {
                items.push("Reset unsupported desktop settings to defaults");
            }
        }
        items.extend(["Save without approving", "Cancel"]);
        let choice = Select::new()
            .with_prompt("Permissions")
            .items(&items)
            .default(1)
            .interact_opt()?;
        match choice.map(|index| items[index]) {
            None | Some("Cancel") => {
                return cli.emit(
                    &serde_json::json!({"updated": false}),
                    "Permissions editing cancelled; no changes saved",
                );
            }
            Some("Game permissions") => {
                let mut game_write = settings.files.game.writable();
                let mut switches = vec![
                    ("Network (internet, LAN, listening)", &mut settings.network),
                    ("Write to game folder", &mut game_write),
                ];
                if kind == Side::Client {
                    switches.push((
                        "Account authentication via host (even when game network is denied)",
                        &mut settings.account_authentication,
                    ));
                    if std::env::consts::OS != "windows" {
                        switches.push((
                            if cfg!(target_os = "linux") {
                                "Audio (includes recording via PulseAudio)"
                            } else {
                                "Audio"
                            },
                            &mut settings.audio,
                        ));
                    }
                }
                edit_switches(&mut switches)?;
                settings.files.game = if game_write {
                    FileAccess::ReadWrite
                } else {
                    FileAccess::ReadOnly
                };
                if !settings.audio && settings.microphone == Some(true) {
                    settings.microphone = None;
                }
            }
            Some("Read-only game folders") => {
                let folders = [
                    GameDirectory::Saves,
                    GameDirectory::Screenshots,
                    GameDirectory::Resourcepacks,
                    GameDirectory::Shaderpacks,
                    GameDirectory::Mods,
                    GameDirectory::Config,
                    GameDirectory::Logs,
                    GameDirectory::Plugins,
                    GameDirectory::World,
                ];
                if let Some(selected) = MultiSelect::new()
                    .with_prompt("Read-only folders — Space to toggle, Enter to keep")
                    .items(
                        folders
                            .iter()
                            .map(|folder| folder.name())
                            .collect::<Vec<_>>(),
                    )
                    .defaults(
                        &folders
                            .iter()
                            .map(|folder| settings.files.read_only.contains(folder))
                            .collect::<Vec<_>>(),
                    )
                    .interact_opt()?
                {
                    settings.files.read_only =
                        selected.into_iter().map(|index| folders[index]).collect();
                }
            }
            Some("External folders") => {
                let mut folders = vec!["Add external folder".to_owned()];
                folders.extend(local.folders.iter().map(|folder| {
                    format!(
                        "Remove {} ({})",
                        clean(&folder.path.display().to_string()),
                        if folder.write {
                            "read/write"
                        } else {
                            "read-only"
                        }
                    )
                }));
                if let Some(selected) = Select::new()
                    .with_prompt("External folders — Esc to return")
                    .items(&folders)
                    .default(0)
                    .interact_opt()?
                {
                    if selected == 0 {
                        let path: String = Input::new()
                            .with_prompt("Existing folder path (empty to cancel)")
                            .allow_empty(true)
                            .interact_text()?;
                        if !path.is_empty()
                            && let Some(access) = Select::new()
                                .with_prompt("Folder access")
                                .items(["Read-only", "Read/write"])
                                .default(0)
                                .interact_opt()?
                            && let Err(error) = allow_folder(
                                ws,
                                &mut local,
                                std::path::Path::new(&path),
                                access == 1,
                            )
                        {
                            eprintln!("{}", clean(&format!("{error:#}")));
                        }
                    } else {
                        local.folders.remove(selected - 1);
                    }
                }
            }
            Some("Advanced desktop permissions") => {
                let mut microphone = settings.microphone == Some(true);
                let mut clipboard = settings.clipboard_allowed();
                let mut switches = vec![
                    (
                        "Import selected files via host confirmation",
                        &mut settings.import_files,
                    ),
                    ("Open web links via host", &mut settings.open_links),
                    (
                        "Open game/approved folders and screenshots via host",
                        &mut settings.open_folders,
                    ),
                    ("Narrator via host", &mut settings.narrator),
                    ("Write skin cache", &mut settings.skin_cache),
                ];
                if std::env::consts::OS != "windows" {
                    switches.push(("Write graphics cache", &mut settings.graphics_cache));
                }
                if cfg!(target_os = "macos") {
                    switches.extend([
                        ("Desktop integration", &mut settings.desktop_integration),
                        ("Microphone (also needs audio)", &mut microphone),
                        ("Clipboard", &mut clipboard),
                    ]);
                }
                edit_switches(&mut switches)?;
                if cfg!(target_os = "macos") {
                    if microphone != (settings.microphone == Some(true)) {
                        settings.microphone = Some(microphone);
                    }
                    if clipboard != (settings.clipboard_allowed()) {
                        settings.clipboard = if clipboard {
                            Clipboard::ReadWrite
                        } else {
                            Clipboard::None
                        };
                    }
                }
                eprintln!(
                    "Only controls supported by this OS are shown. OS limitations also appear before approval."
                );
            }
            Some("Reset unsupported desktop settings to defaults") => {
                settings.reset_unsupported_desktop(std::env::consts::OS);
                eprintln!(
                    "Unsupported desktop settings reset. Review the permissions before approving."
                );
            }
            Some(action @ ("Save and review" | "Save without approving")) => {
                if let Err(error) = settings
                    .validate_for(std::env::consts::OS, kind == Side::Client)
                    .and_then(|_| {
                        enderpin::sandbox::permissions::validate_folders(ws, &local.folders)
                    })
                {
                    eprintln!("Cannot save: {}", clean(&format!("{error:#}")));
                    continue;
                }
                // Saving never grants access. A later review explicitly authorizes the new plan.
                local.approved = None;
                local.save(ws, side)?;
                ws.apply(
                    manifest,
                    ws.lockfile.clone(),
                    &[side],
                    SyncOptions {
                        locked: true,
                        offline: true,
                        lock_only: true,
                        ..Default::default()
                    },
                    |_| {},
                )?;
                eprintln!("Permissions saved; approval is pending.");
                if action == "Save and review" {
                    approve_permissions(cli, ws, side, None)?;
                    println!("Permissions approved");
                }
                return Ok(());
            }
            _ => unreachable!(),
        }
    }
}

fn apply(
    cli: &Cli,
    ws: &mut Workspace,
    manifest: Manifest,
    plan: Lockfile,
    options: SyncOptions,
) -> Result<()> {
    let names = cli.targets(&manifest)?;
    let targets: Vec<_> = names.iter().map(String::as_str).collect();
    let report = ws.apply(manifest, plan, &targets, options, |p| {
        if !cli.json() {
            eprintln!("{}: {} {}", p.target, p.action, clean(&p.package));
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

fn add(cli: &Cli, ws: &mut Workspace, args: &PackageArgs) -> Result<()> {
    let names = cli.targets(&ws.manifest)?;
    let targets: Vec<_> = names.iter().map(String::as_str).collect();
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
                if targets.len() == 1
                    && matches!(
                        ws.manifest.target(targets[0])?.loader.as_str(),
                        "paper" | "velocity"
                    ) =>
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
    for &side in &targets {
        let disabled = manifest.target(side)?.disabled;
        let target = manifest.target_mut(side)?;
        target.packages.insert(name.clone(), package.clone());
        target.disabled = Some(disabled.into_iter().filter(|id| id != &name).collect());
    }
    let options = SyncOptions::default();
    let mut plan = ws.plan(&manifest, &targets, options)?;
    if cli.interactive() && !cli.yes() && args.optional.is_empty() {
        let mut optional = BTreeMap::new();
        for &side in &targets {
            if let Some(target) = plan.targets.get(side) {
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
            for &side in &targets {
                manifest
                    .target_mut(side)?
                    .packages
                    .get_mut(&name)
                    .context("new package missing")?
                    .optional = selected.iter().map(|&i| values[i].0.clone()).collect();
            }
            plan = ws.plan(&manifest, &targets, options)?;
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

fn init(cli: &Cli, args: &Init) -> Result<()> {
    ensure!(
        args.path.is_none() || cli.directory.is_none(),
        "choose enderpin init DIRECTORY or enderpin -C DIRECTORY init; do not specify both"
    );
    let version = args.minecraft.as_deref();
    let loader = args.loader.as_deref();
    let root = args
        .path
        .as_deref()
        .unwrap_or_else(|| cli.workspace_directory());
    if enderpin::storage::read_optional(root, "enderpin.toml")?.is_some() {
        ensure!(
            args.minecraft.is_none()
                && args.loader.is_none()
                && args.loader_version.is_none()
                && args.mods.is_empty()
                && args.kind.is_none(),
            "edit the existing enderpin.toml before initializing it; creation options cannot replace it"
        );
        let cache = cli
            .cache_dir
            .clone()
            .map(Ok)
            .unwrap_or_else(Cache::default_path)?;
        let mut ws = Workspace::open(root, &cache)?;
        let names = cli.targets(&ws.manifest)?;
        let targets: Vec<_> = names.iter().map(String::as_str).collect();
        ws.ensure_layout(&targets)?;
        ws.prepare_runtime(&targets, SyncOptions::default(), false, |name, message| {
            eprintln!("{name}: {}", clean(message))
        })?;
        let created = enderpin::initialize::configs(&mut ws, &targets, false, |message| {
            eprintln!("{}", clean(message))
        })?;
        ws.sync(
            ws.manifest.clone(),
            &targets,
            SyncOptions {
                locked: true,
                ..Default::default()
            },
            |_| {},
        )?;
        println!("Initialized native configuration: {}", created.join(", "));
        return Ok(());
    }
    let scope = if args.target == "all" {
        ensure!(args.kind.is_none(), "--type selects a single target");
        Scope::All
    } else {
        match args.kind {
            Some(enderpin::model::TargetType::Client) => Scope::Client,
            Some(_) => Scope::Server,
            None if args.target == "client" => Scope::Client,
            None if args.target == "server" || args.loader.as_deref() == Some("velocity") => {
                Scope::Server
            }
            None => bail!("creating a named target requires --type client, server or proxy"),
        }
    };
    ensure!(
        cli.interactive() || (version.is_some() && loader.is_some()),
        "init needs a terminal or explicit choices; use enderpin init --minecraft VERSION --loader vanilla|fabric|paper|velocity --no-interactive"
    );
    let root = args
        .path
        .as_deref()
        .unwrap_or_else(|| cli.workspace_directory());
    ensure!(
        enderpin::storage::read_optional(root, "enderpin.toml")?.is_none()
            && enderpin::storage::read_optional(root, "enderpin.lock")?.is_none(),
        "workspace already exists; use enderpin add, sync or run, or enderpin init <NEW_DIRECTORY>"
    );
    let interactive = cli.interactive() && (version.is_none() || loader.is_none());
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
        let mut loaders = vec!["vanilla", "fabric"];
        if matches!(scope, Scope::Server) {
            loaders.extend(["paper", "velocity"]);
        }
        let labels = loaders
            .iter()
            .map(|name| name.to_string())
            .collect::<Vec<_>>();
        loaders[choose("Mod loader", &labels, 0)?].to_owned()
    };
    ensure!(
        !matches!(scope, Scope::All) || !matches!(loader.as_str(), "paper" | "velocity"),
        "Paper and Velocity require a single server/proxy target"
    );
    let loader_version = if loader == "velocity" && args.loader_version.is_none() {
        Some(
            Input::<String>::new()
                .with_prompt("Velocity version (version or version+build)")
                .validate_with(|version: &String| identifier(version))
                .interact_text()?,
        )
    } else {
        args.loader_version.clone()
    };
    ensure!(
        args.mods.is_empty() || loader == "fabric",
        "--mods requires --loader fabric; use enderpin add for plugins or other packages"
    );
    let mut manifest = Workspace::quick_manifest(minecraft.clone())?;
    for side in scope.sides() {
        let target = manifest.target_mut(side.name())?;
        target.loader = loader.clone();
        target.loader_version = loader_version.clone();
        if loader == "velocity" {
            target.kind = Some(enderpin::model::TargetType::Proxy);
        }
    }
    manifest.validate()?;
    let mut registry = enderpin::registry::Modrinth::default();
    let mut selected = BTreeMap::new();
    for id in &args.mods {
        let (name, package) = setup_mod(&mut registry, id, &minecraft, &loader, scope)?;
        selected.insert(name, package);
    }
    if interactive && !cli.yes() && loader == "fabric" {
        choose_mods(&mut registry, &minecraft, &loader, scope, &mut selected)?;
    }
    eprintln!(
        "\nMinecraft {minecraft} / {loader} / {}\nMods: {}\nDirectory: {}",
        match scope {
            Scope::Client => "client",
            Scope::Server => "server",
            Scope::All => "client + server",
        },
        if selected.is_empty() {
            "none".into()
        } else {
            selected.keys().cloned().collect::<Vec<_>>().join(", ")
        },
        root.display()
    );
    if interactive && !cli.yes() {
        ensure!(
            Confirm::new()
                .with_prompt("Create workspace and download files?")
                .default(true)
                .interact()?,
            "setup cancelled"
        );
    }
    let mut names: Vec<String> = scope
        .sides()
        .iter()
        .map(|side| side.name().to_owned())
        .collect();
    manifest.targets.retain(|name, _| names.contains(name));
    if matches!(scope, Scope::All) && !selected.is_empty() {
        manifest.groups.insert(
            "common".into(),
            enderpin::model::Group { packages: selected },
        );
        for name in &names {
            manifest.target_mut(name)?.groups = Some(vec!["common".into()]);
        }
    } else {
        for name in &names {
            manifest.target_mut(name)?.packages = selected.clone();
        }
    }
    if args.target != "all" {
        let mut entry = manifest
            .targets
            .remove(&names[0])
            .context("initial target missing")?;
        if let Some(kind) = args.kind {
            entry.kind = Some(kind);
        }
        manifest.targets.insert(args.target.clone(), entry);
        names = vec![args.target.clone()];
    }
    let names: Vec<_> = names.iter().map(String::as_str).collect();
    Workspace::init_manifest(root, manifest, &names)?;
    let cache = cli
        .cache_dir
        .clone()
        .map(Ok)
        .unwrap_or_else(Cache::default_path)?;
    let mut ws = Workspace::open(root, &cache)?;
    ws.registry = registry;
    ws.prepare_runtime(&names, SyncOptions::default(), false, |side, message| {
        eprintln!("{}: {}", side, clean(message));
    })
    .with_context(|| {
        format!(
            "workspace saved in {}; from that directory retry enderpin sync --target {}",
            root.display(),
            match scope {
                Scope::Client => "client",
                Scope::Server => "server",
                Scope::All => "all",
            }
        )
    })?;
    let created = enderpin::initialize::configs(&mut ws, &names, false, |message| {
        eprintln!("{}", clean(message))
    })?;
    ws.sync(
        ws.manifest.clone(),
        &names,
        SyncOptions {
            locked: true,
            ..Default::default()
        },
        |_| {},
    )?;
    eprintln!("Native configuration: {}", created.join(", "));
    eprintln!("Setup complete. Workspace: {}", root.display());
    eprintln!(
        "From that directory, run: enderpin run --target {}",
        names[0]
    );
    if !matches!(scope, Scope::Client) && loader != "velocity" {
        eprintln!(
            "Server: review the Minecraft EULA, then run enderpin run --target {} --accept-eula.",
            names[0]
        );
    }
    Ok(())
}

fn run(cli: &Cli) -> Result<()> {
    if let Command::Wrapper { version, force } = &cli.command {
        let root = cli.workspace_directory();
        enderpin::wrapper::check_destination(root, *force)?;
        eprintln!("Verifying Enderpin release assets for all supported platforms...");
        enderpin::wrapper::Wrapper::download(version.as_deref())?.write(root, *force)?;
        eprintln!(
            "Created enderpinw, enderpinw.ps1 and enderpin-wrapper.properties. Commit all three files."
        );
        return Ok(());
    }
    if let Command::Init(args) = &cli.command {
        return init(cli, args);
    }
    if let Command::Login { client_id } = &cli.command {
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
    let cache = cli
        .cache_dir
        .clone()
        .map(Ok)
        .unwrap_or_else(Cache::default_path)?;
    let mut ws = Workspace::open(cli.workspace_directory(), &cache)?;
    let names = cli.targets(&ws.manifest)?;
    let targets: Vec<_> = names.iter().map(String::as_str).collect();
    match &cli.command {
        Command::Permissions { action, .. } => {
            use enderpin::sandbox::permissions::{self, Local};
            let side = cli.single_target()?;
            let mut local = Local::load(&ws, side)?;
            match action {
                None => return edit_permissions(cli, &mut ws, side),
                Some(PermissionCommand::Show) => {
                    let plan = permissions::plan(&ws, side)?;
                    let fingerprint = plan.fingerprint()?;
                    let approved = local.approved.as_deref() == Some(&fingerprint);
                    return cli.emit(&serde_json::json!({"fingerprint": fingerprint, "approved": approved, "plan": plan}), format!("{}\nApproval: {}{}", permission_text(&plan), if approved { "current" } else { "needed" }, if approved { String::new() } else { format!("\nReview and approve: enderpin permissions approve --target {}", side) }));
                }
                Some(PermissionCommand::Approve { fingerprint }) => {
                    approve_permissions(cli, &ws, side, fingerprint.as_deref())?;
                    return cli.emit(
                        &serde_json::json!({"updated": true, "approved": true}),
                        "Permissions approved",
                    );
                }
                Some(PermissionCommand::Revoke) => local.approved = None,
                Some(PermissionCommand::AllowFolder { path, write }) => {
                    allow_folder(&ws, &mut local, path, *write)?
                }
                Some(PermissionCommand::RemoveFolder { path }) => {
                    // Also permit removing a saved absolute path after its directory disappeared.
                    let path = path.canonicalize().unwrap_or_else(|_| path.clone());
                    ensure!(
                        local.folders.iter().any(|folder| folder.path == path),
                        "folder is not granted; inspect permissions or use its saved absolute path"
                    );
                    local.folders.retain(|folder| folder.path != path);
                    local.approved = None;
                }
            }
            local.save(&ws, side)?;
            cli.emit(
                &serde_json::json!({"updated": true, "approved": local.approved.is_some()}),
                "Local permissions updated",
            )
        }
        Command::Run {
            service,
            port,
            no_sandbox,
            offline,
            no_network,
            accept_eula,
            memory,
            demo,
            connect,
            jvm_args,
            ..
        } => {
            let side = cli.single_target()?;
            let config = ws.manifest.target(side)?;
            let kind = config.side();
            ensure!(
                !*service || kind == Side::Server,
                "--service is only for servers"
            );
            let stop_command = if ws.manifest.target(side)?.loader == "velocity" {
                "end"
            } else {
                "stop"
            };
            ensure!(
                port.is_none() || kind == Side::Server,
                "--port is only for servers"
            );
            ensure!(!*demo || kind == Side::Client, "--demo is only for clients");
            ensure!(
                !*accept_eula || kind == Side::Server,
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
            let local = enderpin::sandbox::permissions::Local::load(&ws, side)?;
            if !no_sandbox && local.approved.as_deref() != Some(&plan.fingerprint()?) {
                approve_permissions(cli, &ws, side, None)?;
            }
            let stopping = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let signal = stopping.clone();
            ctrlc::set_handler(move || {
                signal.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            })?;
            let session = if kind == Side::Client && !*demo && plan.effective.account_authentication
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
                    memory_mib: memory.unwrap_or(config.memory_mib),
                    port: *port,
                    connect: connect.clone(),
                    jvm_args: jvm_args.clone(),
                },
                session.as_ref(),
                *demo,
                |message| eprintln!("{}: {}", side, clean(message)),
            )?;
            eprintln!(
                "{} {} started (PID {}). Press Ctrl-C to stop.",
                if *no_sandbox {
                    "Unconfined"
                } else {
                    "Sandboxed"
                },
                side,
                game.id()
            );
            let (sender, receiver) = std::sync::mpsc::channel();
            if kind == Side::Server && !service {
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
                    let _ = sender.send(stop_command.into());
                });
            }
            let mut stop_requested = false;
            loop {
                if let Some(status) = game.try_wait()? {
                    ensure!(
                        status.success() || (kind == Side::Client && stop_requested),
                        "{} exited with {status}",
                        side
                    );
                    return Ok(());
                }
                if stopping.load(std::sync::atomic::Ordering::Relaxed) > 0 && !stop_requested {
                    game.stop(kind)?;
                    stop_requested = true;
                    eprintln!("Waiting for shutdown (Ctrl-C again forces termination).");
                }
                while let Ok(line) = receiver.try_recv() {
                    game.console(&line)?;
                    if line.trim() == stop_command {
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
        Command::Sync { .. } | Command::Update { .. } => {
            let options = match &cli.command {
                Command::Sync { options, .. } => SyncOptions {
                    locked: options.locked,
                    offline: options.offline,
                    no_ignore: options.no_ignore,
                    ..Default::default()
                },
                _ => SyncOptions {
                    update: true,
                    ..Default::default()
                },
            };
            let prepared =
                ws.prepare_runtime(&targets, options, options.update, |side, message| {
                    if !cli.json() {
                        eprintln!("{}: {}", side, clean(message));
                    }
                })?;
            let summary: Vec<_> = prepared.iter().map(|(side, runtime)| serde_json::json!({"target": side, "java": runtime.java, "classpath_entries": runtime.classpath.len(), "assets": runtime.assets})).collect();
            cli.emit(&summary, "Runtimes, packages and shared files synchronized")
        }
        Command::Init(_) | Command::Wrapper { .. } | Command::Login { .. } | Command::Logout => {
            unreachable!()
        }
        Command::Add { package, .. } => add(cli, &mut ws, package),
        Command::Remove { name, .. } => {
            identifier(name)?;
            let mut manifest = ws.manifest.clone();
            let mut found = false;
            for &side in &targets {
                let effective = manifest.target(side)?;
                if effective.requests.contains_key(name) {
                    found = true;
                    let target = manifest.target_mut(side)?;
                    target.packages.remove(name);
                    let mut disabled = effective.disabled;
                    if !disabled.contains(name) {
                        disabled.push(name.clone());
                    }
                    target.disabled = Some(disabled);
                }
            }
            ensure!(
                found,
                "{name} is not a direct package in the selected targets"
            );
            let options = SyncOptions::default();
            let plan = ws.plan(&manifest, &targets, options)?;
            apply(cli, &mut ws, manifest, plan, options)
        }
        Command::Search {
            query,
            kind,
            offset,
            ..
        } => {
            let side = cli.single_target()?;
            let results = ws.registry.search(
                query,
                &ws.manifest.target(side)?.minecraft,
                &ws.manifest.target(side)?.loader,
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
                    side,
                    clean(&ws.manifest.target(side)?.minecraft),
                    clean(&ws.manifest.target(side)?.loader)
                ))
                .items(&rows)
                .interact_opt()?
            else {
                return Ok(());
            };
            let hit = &results.hits[index];
            let project = ws.registry.project(&hit.project_id)?;
            let config = ws.manifest.target(side)?;
            let version = ws.registry.versions(&project.id)?.into_iter().find(|v| {
                v.version_type == "release"
                    && v.compatible(&config.minecraft, &config.loader, *kind)
            });
            println!(
                "{}\n{}\nType: {:?}\nTarget: {} / {} / {}\nVersion: {}\n",
                clean(&project.title),
                clean(&project.description),
                kind,
                side,
                clean(&ws.manifest.target(side)?.minecraft),
                clean(&ws.manifest.target(side)?.loader),
                version
                    .as_ref()
                    .map(|v| clean(&v.version_number))
                    .unwrap_or_else(|| "No compatible release".into())
            );
            if version.is_none()
                || (!cli.yes()
                    && !Confirm::new()
                        .with_prompt("Add this project?")
                        .default(false)
                        .interact()?)
            {
                return Ok(());
            }
            add(
                cli,
                &mut ws,
                &PackageArgs {
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
        Command::List { .. } => {
            let mut rows = vec![];
            for &side in &targets {
                if let Some(target) = ws.lockfile.targets.get(side) {
                    let stale = target.fingerprint != ws.manifest.fingerprint(side)?;
                    for package in &target.packages {
                        rows.push(
                            serde_json::json!({"target": side, "stale": stale, "package": package}),
                        );
                    }
                    if !cli.json() {
                        println!("{}{}", side, if stale { " (lock is stale)" } else { "" });
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
            if cli.json() {
                cli.emit(&rows, "")?;
            }
            Ok(())
        }
        command => {
            let options = match command {
                Command::Lock { offline, .. } => SyncOptions {
                    offline: *offline,
                    lock_only: true,
                    ..Default::default()
                },
                Command::Restore {
                    offline, no_ignore, ..
                } => SyncOptions {
                    locked: true,
                    restore: true,
                    offline: *offline,
                    no_ignore: *no_ignore,
                    ..Default::default()
                },
                _ => unreachable!(),
            };
            let manifest = ws.manifest.clone();
            let plan = ws.plan(&manifest, &targets, options)?;
            apply(cli, &mut ws, manifest, plan, options)
        }
    }
}

fn main() {
    let cli = Cli::parse();
    if let Err(error) = run(&cli) {
        if cli.json() {
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
    use clap::CommandFactory;

    #[test]
    fn init_validates_before_writing_and_never_overwrites() -> Result<()> {
        let root = tempfile::tempdir()?;
        let destination = root.path().join("new workspace");
        let directory = destination.to_str().context("non-UTF8 temporary path")?;
        for flags in [
            vec![],
            vec!["--minecraft", "1.21.1"],
            vec!["--loader", "vanilla"],
            vec!["--minecraft", "../bad", "--loader", "vanilla"],
            vec![
                "--minecraft",
                "1.21.1",
                "--loader",
                "vanilla",
                "--mods",
                "sodium",
            ],
            vec![
                "--minecraft",
                "1.21.1",
                "--loader",
                "vanilla",
                "--loader-version",
                "1",
            ],
            vec![
                "--minecraft",
                "1.21.1",
                "--loader",
                "fabric",
                "--loader-version",
                "../bad",
            ],
            vec!["--minecraft", "1.21.1", "--loader", "paper"],
            vec![
                "--minecraft",
                "1.21.1",
                "--loader",
                "velocity",
                "--loader-version",
                "3.4.0-SNAPSHOT+563",
                "--target",
                "all",
            ],
        ] {
            let cli = Cli::try_parse_from(
                [
                    vec!["enderpin", "init", directory, "--no-interactive", "--yes"],
                    flags,
                ]
                .concat(),
            )?;
            assert!(run(&cli).is_err());
            assert!(!destination.exists(), "invalid setup must not create files");
        }
        let cli = Cli::try_parse_from([
            "enderpin",
            "-C",
            directory,
            "init",
            directory,
            "--minecraft",
            "1.21.1",
            "--loader",
            "vanilla",
        ])?;
        assert!(
            run(&cli)
                .expect_err("ambiguous destination")
                .to_string()
                .contains("do not specify both")
        );
        assert!(!destination.exists());
        Workspace::init_quick(&destination, "1.21.1".into())?;
        let original = std::fs::read(destination.join("enderpin.toml"))?;
        let cli = Cli::try_parse_from([
            "enderpin",
            "init",
            directory,
            "--no-interactive",
            "--minecraft",
            "1.21.1",
            "--loader",
            "vanilla",
        ])?;
        assert!(run(&cli).is_err());
        assert_eq!(std::fs::read(destination.join("enderpin.toml"))?, original);
        assert!(
            !destination
                .join(".enderpin/client/permissions.toml")
                .exists()
        );
        Ok(())
    }

    #[test]
    fn command_options_and_help_match_their_scope() -> Result<()> {
        Cli::command().debug_assert();
        for args in [
            vec!["run", "--target", "all"],
            vec!["search", "--target", "all"],
            vec!["permissions", "show", "--target", "all"],
            vec!["init", "--version", "1.21.1"],
            vec!["init", "--server"],
            vec!["init", "--loader", "velocity"],
            vec!["init", "--loader-version", "0.16.10"],
            vec!["init", "--json"],
            vec!["run", "--json"],
            vec!["run", "--yes"],
            vec!["run", "--memory", "255"],
            vec!["run", "--jvm-arg", "-Xmx4G"],
            vec!["run", "--jvm-arg", "-javaagent:/tmp/a.jar"],
            vec!["run", "--jvm-arg"],
            vec!["run", "--demo", "--connect", "localhost"],
            vec!["sync", "--update"],
            vec!["lock", "--yes"],
            vec!["permissions", "approve", "--yes"],
            vec!["login", "--target", "server"],
            vec!["quick"],
            vec!["prepare"],
            vec!["launch"],
        ] {
            assert!(
                Cli::try_parse_from([vec!["enderpin"], args.clone()].concat()).is_err(),
                "accepted {args:?}"
            );
        }
        let help = Cli::command().render_help().to_string();
        for heading in [
            "Getting started:",
            "Everyday:",
            "Settings & authentication:",
            "Automation & recovery:",
        ] {
            assert!(help.contains(heading));
        }
        assert!(help.contains("wrapper"));
        let cli = Cli::try_parse_from([
            "enderpin",
            "-C",
            "my workspace",
            "wrapper",
            "--version",
            "0.1.4",
            "--force",
        ])?;
        assert!(
            matches!(cli.command, Command::Wrapper { version: Some(ref v), force: true } if v == "0.1.4")
        );
        for (command, absent) in [
            ("init", "--json"),
            ("run", "--json"),
            ("sync", "--yes"),
            ("login", "--target"),
        ] {
            let mut root = Cli::command();
            let help = root
                .find_subcommand_mut(command)
                .context("command")?
                .render_long_help()
                .to_string();
            assert!(!help.contains(absent), "{command} advertises {absent}");
        }
        let cli = Cli::try_parse_from([
            "enderpin",
            "run",
            "--target",
            "server",
            "--jvm-arg",
            "-XX:+UseZGC",
            "--jvm-arg=-Dfile.encoding=UTF-8",
        ])?;
        assert!(
            matches!(cli.command, Command::Run { ref jvm_args, memory: None, .. } if jvm_args == &["-XX:+UseZGC", "-Dfile.encoding=UTF-8"])
        );
        let cli = Cli::try_parse_from([
            "enderpin",
            "permissions",
            "show",
            "--target",
            "server",
            "--json",
        ])?;
        assert!(cli.target() == "server");
        assert!(cli.json());
        let cli = Cli::try_parse_from([
            "enderpin",
            "init",
            "my world",
            "--target",
            "all",
            "--minecraft",
            "1.21.1",
            "--loader",
            "fabric",
            "--loader-version",
            "0.16.10",
            "--mods",
            "lithium,sodium",
        ])?;
        let Command::Init(args) = cli.command else {
            panic!("expected init")
        };
        assert_eq!(args.mods.len(), 2);
        assert_eq!(args.loader_version.as_deref(), Some("0.16.10"));
        Ok(())
    }

    #[test]
    fn offline_sync_requires_a_runtime_lock_and_explains_recovery() -> Result<()> {
        let root = tempfile::tempdir()?;
        let cache = tempfile::tempdir()?;
        Workspace::init_quick(root.path(), "1.21.1".into())?;
        let original = std::fs::read(root.path().join("enderpin.lock"))?;
        let cli = Cli::try_parse_from([
            "enderpin",
            "-C",
            root.path().to_str().context("root")?,
            "--cache-dir",
            cache.path().to_str().context("cache")?,
            "sync",
            "--target",
            "server",
            "--locked",
            "--offline",
        ])?;
        let error = run(&cli)
            .expect_err("runtime must already be locked")
            .to_string();
        assert!(error.contains("enderpin sync --target server"), "{error}");
        assert_eq!(std::fs::read(root.path().join("enderpin.lock"))?, original);
        Ok(())
    }

    #[test]
    fn cli_parses_target_and_optional_dependencies() -> Result<()> {
        let cli = Cli::try_parse_from([
            "enderpin",
            "add",
            "sodium",
            "--target",
            "all",
            "--optional",
            "a,b",
            "--no-interactive",
            "--yes",
        ])?;
        assert!(cli.target() == "all");
        assert!(cli.yes());
        let Command::Add { package, .. } = cli.command else {
            panic!("expected add")
        };
        assert_eq!(package.optional, vec!["a", "b"]);
        assert_eq!(clean("hello\x1b[2J\u{202e}world"), "hello[2Jworld");
        Ok(())
    }
}
