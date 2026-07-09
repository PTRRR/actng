use clap::{Parser, Subcommand};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use actng_core::{collect, import_dir, write_csv, Encoding, Profile, Source, Suggestion};
use crossterm::ExecutableCommand;

#[derive(Parser)]
#[command(name = "actng")]
#[command(about = "A learned tagging system for bank statement imports", long_about = None)]
struct Cli {
    /// Path to the profile JSON file (env: ACTNG_PROFILE; default: ./actng.json)
    #[arg(short, long)]
    profile: Option<PathBuf>,

    /// Directory to scan/tag
    #[arg(default_value = ".")]
    directory: PathBuf,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Create a new profile file
    Init {
        /// Name of the profile
        #[arg(short, long)]
        name: Option<String>,

        /// Seed with initial tags (comma-separated)
        #[arg(short, long)]
        tags: Option<String>,
    },
    /// Discover and parse bank files, report layout and entry counts
    Scan {
        /// Override directory
        directory: Option<PathBuf>,
    },
    /// Apply profile and output tagged entries
    Tag {
        /// Override directory
        directory: Option<PathBuf>,

        /// Output format: table, csv, json
        #[arg(short, long, default_value = "table")]
        format: String,
    },
    /// Interactively tag entries the model is unsure about
    Review {
        /// Override directory
        directory: Option<PathBuf>,

        /// Walk every entry, including already-tagged ones
        #[arg(short, long)]
        all: bool,
    },
    /// Manage the tag set (add, rm, category, list)
    Tags {
        #[command(subcommand)]
        action: TagAction,
    },
    /// Export tagged dataset and summary
    Export {
        /// Override directory
        directory: Option<PathBuf>,

        /// Output file path
        #[arg(short, long)]
        output: PathBuf,

        /// Include per-category summary
        #[arg(short, long)]
        summary: bool,
    },
    /// Launch the interactive TUI
    Tui {
        /// Override directory
        directory: Option<PathBuf>,
    },
    /// Show profile statistics
    ProfileInfo,
}

#[derive(Subcommand)]
enum TagAction {
    /// Add a new tag
    Add { tag: String },
    /// Remove a tag and its training data
    Rm {
        tag: String,
        /// Skip the confirmation prompt
        #[arg(short, long)]
        yes: bool,
    },
    /// Assign a category to a tag
    Category { tag: String, category: String },
    /// List all tags, categories, and trained counts
    List,
}

/// Atomically save a profile to disk by writing to a temporary file then renaming.
fn save_profile_atomically(path: &Path, profile: &Profile) -> anyhow::Result<()> {
    let temp_path = path.with_extension("tmp");
    profile.save(&temp_path)?;
    fs::rename(temp_path, path)?;
    Ok(())
}

/// Resolve the profile path: an explicit `--profile` flag wins, then the
/// `ACTNG_PROFILE` environment variable, then `./actng.json`.
fn resolve_profile_path(flag: Option<&Path>) -> PathBuf {
    flag.map(Path::to_path_buf)
        .or_else(|| std::env::var_os("ACTNG_PROFILE").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("actng.json"))
}

/// Persist any layouts detected during this run so a re-export from the same
/// bank never re-runs (or mis-runs) auto-detection next time.
fn persist_new_layouts(
    profile_path: &Path,
    profile: &mut Profile,
    imports: &[actng_core::FileImport],
) -> anyhow::Result<()> {
    if profile.remember_layouts(imports) > 0 {
        save_profile_atomically(profile_path, profile)?;
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("Error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> anyhow::Result<ExitCode> {
    let cli = Cli::parse();
    let profile_path = resolve_profile_path(cli.profile.as_deref());

    let exit_code = match cli.command {
        Commands::Init { name, tags } => {
            if profile_path.exists() {
                anyhow::bail!("Profile already exists at {:?}", profile_path);
            }

            let profile_name = name.unwrap_or_else(|| "personal".to_string());
            let mut profile = Profile::new(profile_name);

            if let Some(tags_str) = tags {
                for tag in tags_str
                    .split(',')
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                {
                    profile.add_tag(tag);
                }
            }

            save_profile_atomically(&profile_path, &profile)?;
            println!("Initialized new profile at {:?}", profile_path);
            ExitCode::SUCCESS
        }
        Commands::Tui { directory } => {
            let dir = directory.as_ref().unwrap_or(&cli.directory);
            let profile_path = resolve_profile_path(cli.profile.as_deref());
            let profile = Profile::load(&profile_path)
                .map_err(|e| anyhow::anyhow!("Failed to load profile: {}", e))?;

            let imports = import_dir(dir, &profile)?;
            let dataset = collect(imports.clone());
            let file_details: Vec<_> = imports
                .into_iter()
                .filter_map(|fi| {
                    let imp = fi.result.as_ref().ok()?;
                    Some(actng_tui::app::FileDetail {
                        path: fi.path.clone(),
                        entries: imp.entries.len(),
                        delimiter: imp.delimiter,
                        encoding: imp.encoding,
                        skipped_rows: imp.skipped_rows,
                        layout_remembered: profile.layouts.contains_key(&imp.fingerprint),
                    })
                })
                .collect();

            let mut stdout = std::io::stdout();
            crossterm::terminal::enable_raw_mode()?;
            stdout.execute(crossterm::terminal::EnterAlternateScreen)?;
            let backend = ratatui::backend::CrosstermBackend::new(stdout);
            let mut terminal = ratatui::Terminal::new(backend)?;

            let result = actng_tui::run(&mut terminal, dir, &profile_path);

            crossterm::terminal::disable_raw_mode()?;
            std::io::stdout().execute(crossterm::terminal::LeaveAlternateScreen)?;

            result
                .map(|_| ExitCode::SUCCESS)
                .map_err(|e| anyhow::anyhow!("TUI error: {e}"))?
        }
        Commands::ProfileInfo => {
            let profile = Profile::load(&profile_path)
                .map_err(|e| anyhow::anyhow!("Failed to load profile: {}", e))?;

            println!("Profile: {}", profile.name);
            println!("Version: {}", profile.version);
            println!("Tags: {}", profile.tags.len());
            println!("Remembered Layouts: {}", profile.layouts.len());
            ExitCode::SUCCESS
        }
        Commands::Scan { directory } => {
            let dir = directory.as_ref().unwrap_or(&cli.directory);
            let mut profile = Profile::load(&profile_path)
                .map_err(|e| anyhow::anyhow!("Failed to load profile: {}", e))?;

            let imports = import_dir(dir, &profile)?;

            println!(
                "{:<28} {:<8} {:<6} {:<8} {:<7} {:<11} {:<23} {:<8} Status",
                "File", "Entries", "Delim", "Encoding", "Header", "Layout", "Date range", "Skipped"
            );
            println!("{:-<115}", "");

            for imp in &imports {
                let filename = imp.path.file_name().unwrap_or_default().to_string_lossy();
                match &imp.result {
                    Ok(import) => {
                        let provenance = if profile.layouts.contains_key(&import.fingerprint) {
                            "remembered"
                        } else {
                            "detected"
                        };
                        let delim = match import.delimiter {
                            b',' => ",".to_string(),
                            b';' => ";".to_string(),
                            b'\t' => "tab".to_string(),
                            other => (other as char).to_string(),
                        };
                        let encoding = match import.encoding {
                            Encoding::Utf8 => "utf-8",
                            Encoding::Windows1252 => "cp1252",
                        };
                        let header = if import.profile.has_header {
                            "yes"
                        } else {
                            "no"
                        };
                        let dates: Vec<_> = import.entries.iter().filter_map(|e| e.date).collect();
                        let range = match (dates.iter().min(), dates.iter().max()) {
                            (Some(min), Some(max)) => format!("{min} to {max}"),
                            _ => "-".to_string(),
                        };
                        println!(
                            "{:<28} {:<8} {:<6} {:<8} {:<7} {:<11} {:<23} {:<8} OK",
                            filename,
                            import.entries.len(),
                            delim,
                            encoding,
                            header,
                            provenance,
                            range,
                            import.skipped_rows,
                        );
                    }
                    Err(e) => {
                        println!(
                            "{filename:<28} {:<8} {:<6} {:<8} {:<7} {:<11} {:<23} {:<8} Error: {e}",
                            "-", "-", "-", "-", "-", "-", "-"
                        );
                    }
                }
            }

            persist_new_layouts(&profile_path, &mut profile, &imports)?;
            ExitCode::SUCCESS
        }
        Commands::Tag { directory, format } => {
            let dir = directory.as_ref().unwrap_or(&cli.directory);
            let mut profile = Profile::load(&profile_path)
                .map_err(|e| anyhow::anyhow!("Failed to load profile: {}", e))?;

            let imports = import_dir(dir, &profile)?;
            persist_new_layouts(&profile_path, &mut profile, &imports)?;
            let dataset = collect(imports);
            let result = profile.run(&dataset);

            if format == "csv" {
                println!("date,amount,description,tag");
                for (entry, sugg) in &result.tagged {
                    println!(
                        "{},{},{},{}",
                        entry
                            .date
                            .map(|d| d.to_string())
                            .unwrap_or_else(|| "unknown".to_string()),
                        entry
                            .amount
                            .map(|a| a.to_string())
                            .unwrap_or_else(|| "unknown".to_string()),
                        entry.description,
                        sugg.tag
                    );
                }
            } else if format == "json" {
                println!("{}", serde_json::to_string_pretty(&result.tagged)?);
            } else {
                println!(
                    "{:<12} {:<10} {:<30} {:<15}",
                    "Date", "Amount", "Description", "Tag"
                );
                println!("{:-<67}", "");
                for (entry, sugg) in &result.tagged {
                    println!(
                        "{:<12} {:<10.2} {:<30} {:<15}",
                        entry
                            .date
                            .map(|d| d.to_string())
                            .unwrap_or_else(|| "unknown".to_string()),
                        entry.amount.unwrap_or(0.0),
                        entry.description.chars().take(28).collect::<String>(),
                        sugg.tag
                    );
                }
            }

            let exact = result
                .tagged
                .iter()
                .filter(|(_, s)| s.source == Source::Exact)
                .count();
            let bayes = result
                .tagged
                .iter()
                .filter(|(_, s)| s.source == Source::Bayes)
                .count();
                let summary = format!(
                    "{} tagged ({exact} exact, {bayes} bayes), {} need review",
                    result.tagged.len(),
                    result.review.len(),
                );

            if format == "table" {
                println!("\n{summary}");
            } else {
                eprintln!("{summary}");
            }

            if result.review.is_empty() {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(2)
            }
        }
        Commands::Review { directory, all } => {
            let dir = directory.as_ref().unwrap_or(&cli.directory);
            let mut profile = Profile::load(&profile_path)
                .map_err(|e| anyhow::anyhow!("Failed to load profile: {}", e))?;

            let imports = import_dir(dir, &profile)?;
            persist_new_layouts(&profile_path, &mut profile, &imports)?;
            let dataset = collect(imports);
            let result = profile.run(&dataset);

            let mut queue = result.review;
            if all {
                for (entry, sugg) in result.tagged {
                    queue.push(actng_core::profile::Review {
                        entry,
                        candidates: vec![(sugg.tag, sugg.confidence)],
                    });
                }
            }

            if queue.is_empty() {
                println!("No entries requiring review.");
                ExitCode::SUCCESS
            } else {
                let total = queue.len();
                println!("Reviewing {total} entries...");

                let mut quit_early = false;
                for (i, item) in queue.into_iter().enumerate() {
                    let date = item
                        .entry
                        .date
                        .map(|d| d.to_string())
                        .unwrap_or_else(|| "unknown".to_string());
                    let amount = item
                        .entry
                        .amount
                        .map(|a| format!("{a:.2}"))
                        .unwrap_or_else(|| "unknown".to_string());
                    println!(
                        "\n[{}/{total}] {date}  {amount}  {}",
                        i + 1,
                        item.entry.description
                    );

                    let mut options: Vec<String> = item
                        .candidates
                        .iter()
                        .map(|(tag, conf)| format!("{tag} ({:.0}%)", conf * 100.0))
                        .collect();
                    let new_tag_idx = options.len();
                    options.push("new tag…".to_string());
                    let skip_idx = options.len();
                    options.push("skip".to_string());
                    let quit_idx = options.len();
                    options.push("quit".to_string());

                    let selection = dialoguer::Select::new()
                        .with_prompt("Tag")
                        .items(&options)
                        .default(0)
                        .interact()?;

                    if selection == quit_idx {
                        quit_early = true;
                        break;
                    } else if selection == skip_idx {
                        continue;
                    } else if selection == new_tag_idx {
                        let name: String = dialoguer::Input::new()
                            .with_prompt("New tag name")
                            .interact_text()?;
                        let name = name.trim();
                        if name.is_empty() {
                            println!("Empty tag name, skipping.");
                            continue;
                        }
                        profile.learn(&item.entry.description, name);
                        save_profile_atomically(&profile_path, &profile)?;
                        println!("Tagged as: {name}");
                    } else {
                        let tag = item.candidates[selection].0.clone();
                        profile.learn(&item.entry.description, &tag);
                        save_profile_atomically(&profile_path, &profile)?;
                        println!("Tagged as: {tag}");
                    }
                }

                if quit_early {
                    println!("\nStopped early; profile saved with answers so far.");
                } else {
                    println!("\nReview complete; profile saved.");
                }
                ExitCode::SUCCESS
            }
        }
        Commands::Tags { action } => match action {
            TagAction::Add { tag } => {
                let mut profile = Profile::load(&profile_path)
                    .map_err(|e| anyhow::anyhow!("Failed to load profile: {}", e))?;
                profile.add_tag(&tag);
                save_profile_atomically(&profile_path, &profile)?;
                println!("Added tag: {}", tag);
                ExitCode::SUCCESS
            }
            TagAction::Rm { tag, yes } => {
                let mut profile = Profile::load(&profile_path)
                    .map_err(|e| anyhow::anyhow!("Failed to load profile: {}", e))?;

                let trained = profile
                    .tagger
                    .stats()
                    .into_iter()
                    .find(|s| s.tag == tag)
                    .map(|s| s.trained_docs)
                    .unwrap_or(0);

                let confirmed = yes
                    || dialoguer::Confirm::new()
                        .with_prompt(format!(
                            "Delete '{tag}' and its {trained} trained document{}? This cannot be undone.",
                            if trained == 1 { "" } else { "s" }
                        ))
                        .default(false)
                        .interact()?;

                if confirmed {
                    profile.remove_tag(&tag);
                    save_profile_atomically(&profile_path, &profile)?;
                    println!("Removed tag: {}", tag);
                } else {
                    println!("Aborted; profile unchanged.");
                }
                ExitCode::SUCCESS
            }
            TagAction::Category { tag, category } => {
                let mut profile = Profile::load(&profile_path)
                    .map_err(|e| anyhow::anyhow!("Failed to load profile: {}", e))?;
                profile
                    .set_category(&tag, category.clone())
                    .map_err(|e| anyhow::anyhow!("Error setting category: {}", e))?;
                save_profile_atomically(&profile_path, &profile)?;
                println!("Assigned category {} to tag {}", category, tag);
                ExitCode::SUCCESS
            }
            TagAction::List => {
                let profile = Profile::load(&profile_path)
                    .map_err(|e| anyhow::anyhow!("Failed to load profile: {}", e))?;
                let stats = profile.tagger.stats();
                println!(
                    "{:<15} {:<15} {:<10} {:<10}",
                    "Tag", "Category", "Trained", "Exact"
                );
                println!("{:-<50}", "");
                for t in &profile.tags {
                    let s = stats.iter().find(|s| s.tag == t.name);
                    println!(
                        "{:<15} {:<15} {:<10} {:<10}",
                        t.name,
                        t.category.as_deref().unwrap_or("-"),
                        s.map(|s| s.trained_docs).unwrap_or(0),
                        s.map(|s| s.exact_keys).unwrap_or(0),
                    );
                }
                ExitCode::SUCCESS
            }
        },
        Commands::Export {
            directory,
            output,
            summary,
        } => {
            let dir = directory.as_ref().unwrap_or(&cli.directory);
            let mut profile = Profile::load(&profile_path)
                .map_err(|e| anyhow::anyhow!("Failed to load profile: {}", e))?;

            let imports = import_dir(dir, &profile)?;
            persist_new_layouts(&profile_path, &mut profile, &imports)?;
            let dataset = collect(imports);

            let suggestions: Vec<Option<Suggestion>> = dataset
                .entries
                .iter()
                .map(|e| profile.suggest(&e.description))
                .collect();

            let file = fs::File::create(&output)?;
            let export_summary = write_csv(file, &dataset, &profile, &suggestions)?;

            if summary {
                println!("\n--- Export Summary ---");
                println!("{:<20} {:<10}", "Category", "Total");
                println!("{:-<30}", "");
                let mut sorted_cats = export_summary.per_category.clone();
                sorted_cats.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
                for (cat, total) in sorted_cats {
                    println!("{:<20} {:<10.2}", cat, total);
                }
            }

            println!("Exported {} entries to {:?}", export_summary.rows, output);
            ExitCode::SUCCESS
        }
    };

    Ok(exit_code)
}
