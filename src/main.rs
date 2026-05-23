use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{ArgAction, Parser, Subcommand};
use symcfg::DEFAULT_CONFIG_FILENAME;
use symcfg::apply::{self, ApplyDecision, ApplyOptions, ApplyPrompter};
use symcfg::config::LinkEntry;
use symcfg::link::{self, LinkOptions, ParentDecision, ParentPrompter};
use symcfg::sync::{self, AutoDeletePolicy, SyncDeleteDecision, SyncOptions, SyncPrompter};

#[derive(Debug, Parser)]
#[command(name = "symcfg")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Search {
        #[arg(long = "in", required = true, num_args = 1..)]
        link_roots: Vec<PathBuf>,

        #[arg(long = "source", default_value = ".")]
        source_root: PathBuf,

        #[arg(short = 'o', long = "output", default_value = DEFAULT_CONFIG_FILENAME)]
        output: PathBuf,
    },
    Link {
        src: PathBuf,
        link: PathBuf,

        #[arg(short = 'c', long = "config", default_value = DEFAULT_CONFIG_FILENAME)]
        config: PathBuf,

        #[arg(short = 'y', long = "yes", action = ArgAction::SetTrue)]
        yes: bool,
    },
    Apply {
        #[arg(short = 'c', long = "config", default_value = DEFAULT_CONFIG_FILENAME)]
        config: PathBuf,

        #[arg(short = 'y', long = "yes", action = ArgAction::SetTrue)]
        yes: bool,
    },
    List {
        #[arg(short = 'c', long = "config", default_value = DEFAULT_CONFIG_FILENAME)]
        config: PathBuf,
    },
    Sync {
        #[arg(long = "source", default_value = ".")]
        source_root: PathBuf,

        #[arg(short = 'c', long = "config", default_value = DEFAULT_CONFIG_FILENAME)]
        config: PathBuf,

        #[arg(short = 'y', long = "yes", action = ArgAction::SetTrue)]
        yes: bool,

        #[arg(long = "delete-links", conflicts_with = "keep_links", action = ArgAction::SetTrue)]
        delete_links: bool,

        #[arg(long = "keep-links", action = ArgAction::SetTrue)]
        keep_links: bool,
    },
    Validate {
        #[arg(short = 'c', long = "config", default_value = DEFAULT_CONFIG_FILENAME)]
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
            println!(
                "Search complete: matched={}, added={}, duplicate={}, conflict={}",
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
            println!(
                "Link complete: created={}, parent_created={}, registered={}, duplicate={}",
                report.created_link, report.created_parent, report.registered, report.duplicate
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
            println!(
                "Apply complete: created={}, skipped={}, conflict={}",
                report.created,
                report.skipped,
                report.conflicts.len()
            );
        } // LCOV_EXCL_LINE
        Command::List { config } => {
            let items = symcfg::list::list_config(&config).map_err(|err| err.to_string())?;
            for item in items {
                println!(
                    "{}\t{}\t{}",
                    item.status.as_str(),
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
            println!(
                "Sync complete: stale={}, removed={}, deleted={}, kept={}",
                report.stale, report.removed_entries, report.deleted_links, report.kept_links
            );
        }
        Command::Validate { config } => {
            apply::validate_config_file(&config).map_err(|err| format!("invalid config: {err}"))?;
            println!("Config is valid");
        }
    }

    Ok(())
}

fn prompt_yes_no(message: &str) -> io::Result<bool> {
    print!("{message}");
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    Ok(matches!(input.trim(), "y" | "Y") || input.trim().eq_ignore_ascii_case("yes"))
}
