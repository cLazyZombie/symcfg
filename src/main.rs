use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anstream::println;
use anstyle::{AnsiColor, Style};
use clap::{ArgAction, Parser, Subcommand};
use symcfg::DEFAULT_CONFIG_FILENAME;
use symcfg::apply::{
    self, ApplyDecision, ApplyItemStatus, ApplyOptions, ApplyPrompter, ApplySkipReason,
};
use symcfg::config::LinkEntry;
use symcfg::import::{
    self, ImportDecision, ImportItemStatus, ImportOptions, ImportPrompter, ImportSkipReason,
};
use symcfg::link::{self, LinkOptions, ParentDecision, ParentPrompter};
use symcfg::list::LinkStatus;
use symcfg::search::SearchItem;
use symcfg::sync::{self, AutoDeletePolicy, SyncDeleteDecision, SyncOptions, SyncPrompter};
use symcfg::sync::{SyncItemStatus, SyncReport};

#[derive(Debug, Parser)]
#[command(
    name = "symcfg",
    about = "Manage configuration files through symbolic links.",
    long_about = "Manage configuration files through symbolic links.\n\nsymcfg keeps real configuration files in a source directory and manages symbolic links at the paths read by applications.\n\nUse search when links already exist, import when a real file is still at the application path, link when the source file already exists, apply to recreate missing links from symbolic.json, list to inspect current status, sync to prune stale entries, and validate to check the config file."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    #[command(
        about = "Find existing symlinks and add them to the config.",
        long_about = "Scan one or more link roots for symbolic links whose targets live under the source root, then add those link -> src pairs to symbolic.json.\n\nThis command does not create or modify symlinks. It is useful after you already moved files into a source directory and created symlinks by hand."
    )]
    Search {
        #[arg(help = "Directories to scan for existing symbolic links.")]
        #[arg(required = true, num_args = 1..)]
        link_roots: Vec<PathBuf>,

        #[arg(
            long = "source",
            default_value = ".",
            help = "Directory that contains the real source files symlinks should point into."
        )]
        source_root: PathBuf,

        #[arg(
            short = 'o',
            long = "output",
            default_value = DEFAULT_CONFIG_FILENAME,
            help = "Config file to create or update."
        )]
        output: PathBuf,
    },
    #[command(
        about = "Create a symlink for an existing source file.",
        long_about = "Create LINK as a symbolic link pointing to SRC, then register the pair in symbolic.json.\n\nUse this when the managed source file already exists. The command never overwrites a regular file or a symlink that points somewhere else. If LINK's parent directory is missing, symcfg asks before creating it unless --yes is provided."
    )]
    Link {
        #[arg(help = "Existing source file or directory that should be the symlink target.")]
        src: PathBuf,
        #[arg(help = "Application path where the symbolic link should be created.")]
        link: PathBuf,

        #[arg(
            short = 'c',
            long = "config",
            default_value = DEFAULT_CONFIG_FILENAME,
            help = "Config file to create or update."
        )]
        config: PathBuf,

        #[arg(
            short = 'y',
            long = "yes",
            action = ArgAction::SetTrue,
            help = "Create missing parent directories without prompting."
        )]
        yes: bool,
    },
    #[command(
        about = "Import an existing regular file and replace it with a symlink.",
        long_about = "Copy the existing regular file at LINK into SRC, replace LINK with a symbolic link pointing to SRC, then register the pair in symbolic.json.\n\nUse this when an application already has a real configuration file at its normal path and you want symcfg to manage it. The command only imports regular files, refuses to overwrite an existing SRC, and asks before changing files unless --yes is provided."
    )]
    Import {
        #[arg(help = "Existing regular file at the application path to import.")]
        link: PathBuf,

        #[arg(help = "New source path where the file should be copied before LINK is replaced.")]
        src: PathBuf,

        #[arg(
            short = 'c',
            long = "config",
            default_value = DEFAULT_CONFIG_FILENAME,
            help = "Config file to create or update."
        )]
        config: PathBuf,

        #[arg(
            short = 'y',
            long = "yes",
            action = ArgAction::SetTrue,
            help = "Import and replace the file without prompting."
        )]
        yes: bool,
    },
    #[command(
        about = "Create missing symlinks from the config.",
        long_about = "Read symbolic.json and create any missing symbolic links recorded in it.\n\nThis command does not create missing parent directories and never overwrites regular files or symlinks that point somewhere else. Without --yes, symcfg asks before each link creation."
    )]
    Apply {
        #[arg(
            short = 'c',
            long = "config",
            default_value = DEFAULT_CONFIG_FILENAME,
            help = "Config file to apply."
        )]
        config: PathBuf,

        #[arg(
            short = 'y',
            long = "yes",
            action = ArgAction::SetTrue,
            help = "Create missing links without prompting."
        )]
        yes: bool,
    },
    #[command(
        about = "Show every configured link and its current status.",
        long_about = "Print each symbolic.json entry with a status label.\n\nlinked means LINK points to SRC, missing means LINK does not exist, and conflict means LINK exists but is not the expected symlink."
    )]
    List {
        #[arg(
            short = 'c',
            long = "config",
            default_value = DEFAULT_CONFIG_FILENAME,
            help = "Config file to inspect."
        )]
        config: PathBuf,
    },
    #[command(
        about = "Remove config entries whose source files disappeared.",
        long_about = "Remove symbolic.json entries whose SRC is under SOURCE_ROOT and no longer exists.\n\nWhen removing a stale entry, symcfg can also delete the matching symlink at LINK. It only deletes symlinks that still point to the recorded SRC; regular files and symlinks to other targets are kept."
    )]
    Sync {
        #[arg(
            help = "Source root whose missing entries should be removed.",
            default_value = "."
        )]
        source_root: PathBuf,

        #[arg(
            short = 'c',
            long = "config",
            default_value = DEFAULT_CONFIG_FILENAME,
            help = "Config file to update."
        )]
        config: PathBuf,

        #[arg(
            short = 'y',
            long = "yes",
            action = ArgAction::SetTrue,
            help = "Run non-interactively; requires --delete-links or --keep-links."
        )]
        yes: bool,

        #[arg(
            long = "delete-links",
            conflicts_with = "keep_links",
            action = ArgAction::SetTrue,
            help = "Delete matching stale symlinks while removing stale config entries."
        )]
        delete_links: bool,

        #[arg(
            long = "keep-links",
            action = ArgAction::SetTrue,
            help = "Keep stale link paths while removing stale config entries."
        )]
        keep_links: bool,
    },
    #[command(
        about = "Validate that the config file can be read.",
        long_about = "Read symbolic.json and verify that it uses a supported schema version with valid link and src fields.\n\nThis command does not inspect the filesystem state of the configured links; use list for that."
    )]
    Validate {
        #[arg(
            short = 'c',
            long = "config",
            default_value = DEFAULT_CONFIG_FILENAME,
            help = "Config file to validate."
        )]
        config: PathBuf,
    },
}

struct StdioPrompter;

impl ParentPrompter for StdioPrompter {
    fn decide_create_parent(&mut self, parent: &Path) -> Result<ParentDecision, link::LinkError> {
        let answer = prompt_yes_no(&format!(
            "Create missing parent directory {parent:?}? [y/N] "
        ))
        .map_err(
            // LCOV_EXCL_START
            |source| link::LinkError::Io {
                path: PathBuf::from("<stdin>"),
                source,
            },
        )?;
        // LCOV_EXCL_STOP

        Ok(if answer {
            ParentDecision::Create
        } else {
            ParentDecision::Skip
        })
    }
}

impl ApplyPrompter for StdioPrompter {
    fn decide_create_link(
        &mut self,
        entry: &LinkEntry,
    ) -> Result<ApplyDecision, apply::ApplyError> {
        let answer = prompt_yes_no(&format!(
            "Create link {:?} pointing to {:?}? [y/N] ",
            entry.link, entry.src
        ))
        .map_err(
            // LCOV_EXCL_START
            |err| apply::ApplyError::Prompt {
                message: err.to_string(),
            },
        )?;
        // LCOV_EXCL_STOP

        Ok(if answer {
            ApplyDecision::Create
        } else {
            ApplyDecision::Skip
        })
    }
}

impl ImportPrompter for StdioPrompter {
    fn decide_import(
        &mut self,
        link: &Path,
        src: &Path,
    ) -> Result<ImportDecision, import::ImportError> {
        let answer = prompt_yes_no(&format!(
            "Import existing file {link:?} into {src:?} and replace it with a symlink? [y/N] "
        ))
        .map_err(
            // LCOV_EXCL_START
            |err| import::ImportError::Prompt {
                message: err.to_string(),
            },
        )?;
        // LCOV_EXCL_STOP

        Ok(if answer {
            ImportDecision::Import
        } else {
            ImportDecision::Skip
        })
    }
}

impl SyncPrompter for StdioPrompter {
    fn decide_delete_link(
        &mut self,
        entry: &LinkEntry,
    ) -> Result<SyncDeleteDecision, sync::SyncError> {
        let answer = prompt_yes_no(&format!(
            "Source {:?} is stale. Delete the link {:?}? [y/N] ",
            entry.src, entry.link
        ))
        .map_err(
            // LCOV_EXCL_START
            |err| sync::SyncError::Prompt(err.to_string()),
        )?;
        // LCOV_EXCL_STOP

        Ok(if answer {
            SyncDeleteDecision::DeleteLink
        } else {
            SyncDeleteDecision::KeepLink
        })
    }
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("Error: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<(), String> {
    match cli.command {
        Command::Search {
            source_root,
            link_roots,
            output,
        } => {
            let report =
                symcfg::search::search_and_update_config(&source_root, &link_roots, &output)
                    .map_err(|err| err.to_string())?;
            for item in &report.items {
                match item {
                    SearchItem::Added { link, src } => {
                        println!("{} {} -> {}", added(), link.display(), src.display());
                    }
                    SearchItem::Duplicate { link, src } => {
                        println!("{} {} -> {}", duplicate(), link.display(), src.display());
                    }
                    SearchItem::Conflict {
                        link,
                        existing_src,
                        new_src,
                    } => {
                        println!(
                            "{} {} existing={} new={}",
                            conflict(),
                            link.display(),
                            existing_src.display(),
                            new_src.display()
                        );
                    }
                }
            }
            println!(
                "{} matched={}, added={}, duplicate={}, conflict={}",
                summary("Search complete"),
                report.matched,
                report.added,
                report.duplicates,
                report.conflicts.len()
            );
        }
        Command::Link {
            src,
            link,
            config,
            yes,
        } => {
            let report = link::link_and_register(
                &src,
                &link,
                &config,
                LinkOptions {
                    yes,
                    prompter: StdioPrompter,
                },
            )
            .map_err(|err| err.to_string())?;
            if report.created_parent
                && let Some(parent) = link.parent()
            {
                println!("{} {}", created_parent(), parent.display());
            }
            if report.created_link {
                println!("{} {} -> {}", created(), link.display(), src.display());
            } else {
                println!(
                    "{} {} -> {}",
                    skipped("already-linked"),
                    link.display(),
                    src.display()
                );
            }
            if report.registered {
                println!("{} {} -> {}", registered(), link.display(), src.display());
            }
            if report.duplicate {
                println!("{} {} -> {}", duplicate(), link.display(), src.display());
            }
            println!(
                "{} created={}, parent_created={}, registered={}, duplicate={}",
                summary("Link complete"),
                report.created_link,
                report.created_parent,
                report.registered,
                report.duplicate
            );
        }
        Command::Import {
            link,
            src,
            config,
            yes,
        } => {
            let report = import::import_and_register(
                &link,
                &src,
                &config,
                ImportOptions {
                    yes,
                    prompter: StdioPrompter,
                },
            )
            .map_err(|err| err.to_string())?;
            if report.created_parent
                && let Some(parent) = report.src.parent()
            {
                println!("{} {}", created_parent(), parent.display());
            }
            match report.status {
                ImportItemStatus::Imported => {
                    println!(
                        "{} {} -> {}",
                        imported(),
                        report.link.display(),
                        report.src.display()
                    );
                    if report.registered {
                        println!(
                            "{} {} -> {}",
                            registered(),
                            report.link.display(),
                            report.src.display()
                        );
                    }
                    if report.duplicate {
                        println!(
                            "{} {} -> {}",
                            duplicate(),
                            report.link.display(),
                            report.src.display()
                        );
                    }
                }
                ImportItemStatus::Skipped(reason) => {
                    println!(
                        "{} {} -> {}",
                        skipped(import_skip_reason(reason)),
                        report.link.display(),
                        report.src.display()
                    );
                }
            }
            println!(
                "{} imported={}, parent_created={}, registered={}, duplicate={}",
                summary("Import complete"),
                report.status == ImportItemStatus::Imported,
                report.created_parent,
                report.status == ImportItemStatus::Imported && report.registered,
                report.status == ImportItemStatus::Imported && report.duplicate
            );
        }
        Command::Apply { config, yes } => {
            let report = apply::apply_config(
                &config,
                ApplyOptions {
                    yes,
                    prompter: StdioPrompter,
                },
            )
            .map_err(|err| err.to_string())?;
            for item in &report.items {
                match item.status {
                    ApplyItemStatus::Created => {
                        println!(
                            "{} {} -> {}",
                            created(),
                            item.link.display(),
                            item.src.display()
                        );
                    }
                    ApplyItemStatus::Skipped(reason) => {
                        println!(
                            "{} {} -> {}",
                            skipped(apply_skip_reason(reason)),
                            item.link.display(),
                            item.src.display()
                        );
                    }
                    ApplyItemStatus::Conflict => {
                        println!(
                            "{} {} -> {}",
                            conflict(),
                            item.link.display(),
                            item.src.display()
                        );
                    }
                }
            }
            println!(
                "{} created={}, skipped={}, conflict={}",
                summary("Apply complete"),
                report.created,
                report.skipped,
                report.conflicts.len()
            );
        } // LCOV_EXCL_LINE
        Command::List { config } => {
            let items = symcfg::list::list_config(&config).map_err(|err| err.to_string())?;
            for item in items {
                println!(
                    "{} {} -> {}",
                    list_status(item.status),
                    item.link.display(),
                    item.src.display()
                );
            }
        }
        Command::Sync {
            source_root,
            config,
            yes,
            delete_links,
            keep_links,
        } => {
            let auto_delete_policy = match (delete_links, keep_links) {
                (true, false) => Some(AutoDeletePolicy::DeleteLinks),
                (false, true) => Some(AutoDeletePolicy::KeepLinks),
                (false, false) => None,
                (true, true) => unreachable!("clap prevents mutually exclusive flags"), // LCOV_EXCL_LINE
            };

            if !yes {
                if delete_links {
                    return Err("--delete-links requires --yes".to_owned());
                }

                if keep_links {
                    return Err("--keep-links requires --yes".to_owned());
                }
            } else if auto_delete_policy.is_none() {
                return Err(
                    "Choose whether to delete stale links with --delete-links or keep stale links with --keep-links when using --yes"
                        .to_owned(),
                );
            }

            let report = sync::sync_config(
                &source_root,
                &config,
                SyncOptions {
                    yes,
                    auto_delete_policy,
                    prompter: StdioPrompter,
                },
            )
            .map_err(|err| err.to_string())?;
            print_sync_items(&report);
            println!(
                "{} stale={}, removed={}, deleted={}, kept={}",
                summary("Sync complete"),
                report.stale,
                report.removed_entries,
                report.deleted_links,
                report.kept_links
            );
        }
        Command::Validate { config } => {
            apply::validate_config_file(&config).map_err(|err| format!("invalid config: {err}"))?;
            println!("{} {}", valid(), config.display());
        }
    }

    Ok(())
}

fn style(color: AnsiColor) -> Style {
    color.on_default().bold()
}

fn label(text: &str, style: Style) -> String {
    format!("{style}{text:<18}{style:#}")
}

fn summary(text: &str) -> String {
    label(text, style(AnsiColor::BrightCyan))
}

fn added() -> String {
    label("added", style(AnsiColor::Green))
}

fn registered() -> String {
    label("registered", style(AnsiColor::Green))
}

fn created() -> String {
    label("created", style(AnsiColor::Green))
}

fn imported() -> String {
    label("imported", style(AnsiColor::Green))
}

fn created_parent() -> String {
    label("created-parent", style(AnsiColor::Green))
}

fn duplicate() -> String {
    label("duplicate", style(AnsiColor::Yellow))
}

fn skipped(reason: &str) -> String {
    label(&format!("skipped:{reason}"), style(AnsiColor::Yellow))
}

fn conflict() -> String {
    label("conflict", style(AnsiColor::Red))
}

fn valid() -> String {
    label("valid", style(AnsiColor::Green))
}

fn apply_skip_reason(reason: ApplySkipReason) -> &'static str {
    match reason {
        ApplySkipReason::AlreadyLinked => "already-linked",
        ApplySkipReason::MissingParent => "missing-parent",
        ApplySkipReason::Declined => "declined",
    }
}

fn import_skip_reason(reason: ImportSkipReason) -> &'static str {
    match reason {
        ImportSkipReason::Declined => "declined",
    }
}

fn list_status(status: LinkStatus) -> String {
    match status {
        LinkStatus::Linked => label("linked", style(AnsiColor::Green)),
        LinkStatus::Missing => label("missing", style(AnsiColor::Yellow)),
        LinkStatus::Conflict => conflict(),
    }
}

fn sync_status(status: SyncItemStatus) -> String {
    match status {
        SyncItemStatus::DeletedLink => label("deleted", style(AnsiColor::Green)),
        SyncItemStatus::KeptLink => label("kept", style(AnsiColor::Yellow)),
        SyncItemStatus::MissingLink => label("missing-link", style(AnsiColor::Yellow)),
    }
}

fn print_sync_items(report: &SyncReport) {
    for item in &report.items {
        println!(
            "{} {} -> {}",
            sync_status(item.status),
            item.link.display(),
            item.src.display()
        );
    }
}

fn prompt_yes_no(message: &str) -> io::Result<bool> {
    print!("{message}");
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    Ok(matches!(input.trim(), "y" | "Y") || input.trim().eq_ignore_ascii_case("yes"))
}
