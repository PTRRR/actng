use clap::{Parser, Subcommand};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use actng_core::{import_dir, Profile};

#[derive(Parser)]
#[command(name = "actng")]
#[command(about = "A learned tagging system for bank statement imports", long_about = None)]
struct Cli {
    /// Path to the profile JSON file
    #[arg(short, long, default_value = "actng.json")]
    profile: PathBuf,

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
    /// Show profile statistics
    ProfileInfo,
}

#[derive(Subcommand)]
enum TagAction {
    /// Add a new tag
    Add { tag: String },
    /// Remove a tag and its training data
    Rm { tag: String },
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

/// Resolve the profile path, checking the environment variable before the CLI flag.
fn resolve_profile_path(cli_path: &Path) -> PathBuf {
    std::env::var("ACTNG_PROFILE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| cli_path.to_path_buf())
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let profile_path = resolve_profile_path(&cli.profile);

    match cli.command {
        Commands::Init { name, tags } => {
            if profile_path.exists() {
                anyhow::bail!("Profile already exists at {:?}", profile_path);
            }

            let profile_name = name.unwrap_or_else(|| "personal".to_string());
            let mut profile = Profile::new(profile_name);

            if let Some(tags_str) = tags {
                for tag in tags_str.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
                    profile.add_tag(tag);
                }
            }

            save_profile_atomically(&profile_path, &profile)?;
            println!("Initialized new profile at {:?}", profile_path);
        }
        Commands::ProfileInfo => {
            let profile = Profile::load(&profile_path)
                .map_err(|e| anyhow::anyhow!("Failed to load profile: {}", e))?;
            
            println!("Profile: {}", profile.name);
            println!("Version: {}", profile.version);
            println!("Tags: {}", profile.tags.len());
            println!("Remembered Layouts: {}", profile.layouts.len());
        }
        Commands::Scan { directory } => {
            let dir = directory.as_ref().unwrap_or(&cli.directory);
            let profile = Profile::load(&profile_path)
                .map_err(|e| anyhow::anyhow!("Failed to load profile: {}", e))?;
            
            let imports = import_dir(dir, &profile)?;
            
            println!("{:<40} {:<10} {:<10}", "File", "Entries", "Status");
            println!("{:-<62}", "");
            
            for imp in imports {
                let filename = imp.path.file_name().unwrap_or_default().to_string_lossy();
                match &imp.result {
                    Ok(import) => {
                        println!("{:<40} {:<10} OK", filename, import.entries.len());
                    }
                    Err(e) => {
                        println!("{:<40} {:<10} Error: {}", filename, "-", e);
                    }
                }
            }
        }
        Commands::Tag { directory, format } => {
            let dir = directory.as_ref().unwrap_or(&cli.directory);
            let profile = Profile::load(&profile_path)
                .map_err(|e| anyhow::anyhow!("Failed to load profile: {}", e))?;

            let imports = import_dir(dir, &profile)?;
            let result = profile.run(&imports);
            
            if format == "csv" {
                println!("date,amount,description,tag");
                for (entry, sugg) in &result.tagged {
                    println!("{},{},{},{}", 
                        entry.date.map(|d| d.to_string()).unwrap_or_else(|| "unknown".to_string()), 
                        entry.amount.map(|a| a.to_string()).unwrap_or_else(|| "unknown".to_string()), 
                        entry.description, 
                        sugg.tag
                    );
                }
            } else if format == "json" {
                println!("{}", serde_json::to_string_pretty(&result.tagged)?);
            } else {
                println!("{:<12} {:<10} {:<30} {:<15}", "Date", "Amount", "Description", "Tag");
                println!("{:-<67}", "");
                for (entry, sugg) in &result.tagged {
                    println!("{:<12} {:<10.2} {:<30} {:<15}", 
                        entry.date.map(|d| d.to_string()).unwrap_or_else(|| "unknown".to_string()), 
                        entry.amount.unwrap_or(0.0), 
                        entry.description.chars().take(28).collect::<String>(), 
                        sugg.tag
                    );
                }
            }
        }
        Commands::Review { directory, all } => {
            let dir = directory.as_ref().unwrap_or(&cli.directory);
            let mut profile = Profile::load(&profile_path)
                .map_err(|e| anyhow::anyhow!("Failed to load profile: {}", e))?;

            let imports = import_dir(dir, &profile)?;
            let result = profile.run(&imports);

            let total_entries = result.tagged.len() + result.review.len();
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
                return Ok(());
            }

            println!("Reviewing {} entries...", queue.len());
            let mut learned_any = false;

            for (i, item) in queue.into_iter().enumerate() {
                println!("\n[{}/{}]", i + 1, total_entries);
                println!("Date: {:?} | Amount: {:?} | Desc: {}", item.entry.date, item.entry.amount, item.entry.description);
                
                println!("Candidates:");
                for (j, (tag, conf)) in item.candidates.iter().enumerate() {
                    println!("  {}. {:<15} ({:.2})", j + 1, tag, conf);
                }
                
                println!("\nQuick Tags (1-9):");
                for (j, tag) in profile.tags.iter().take(9).enumerate() {
                    println!("  {}. {:<15}", j + 1, tag.name);
                }
                println!("  S. Skip");

                print!("Tag (or 's' to skip) > ");
                io::stdout().flush()?;

                let mut input = String::new();
                io::stdin().read_line(&mut input)?;
                let input = input.trim();

                if input.to_lowercase() == "s" {
                    continue;
                }

                if input.is_empty() {
                    println!("Empty input, skipping.");
                    continue;
                }

                if let Ok(idx) = input.parse::<usize>() {
                    if idx > 0 && idx <= 9 && idx <= profile.tags.len() {
                        let tag = profile.tags[idx - 1].name.clone();
                        profile.learn(&item.entry.description, &tag);
                        learned_any = true;
                        save_profile_atomically(&profile_path, &profile)?;
                        println!("Tagged as: {}", tag);
                        continue;
                    }
                }

                // 1. Try exact match or unique prefix match against existing tags
                let matching_tags: Vec<String> = profile.tags.iter()
                    .filter(|t| t.name.to_lowercase().starts_with(&input.to_lowercase()))
                    .map(|t| t.name.clone())
                    .collect();

                if matching_tags.len() == 1 {
                    let tag = &matching_tags[0];
                    profile.learn(&item.entry.description, tag);
                    learned_any = true;
                    save_profile_atomically(&profile_path, &profile)?;
                    println!("Tagged as: {}", tag);
                    continue;
                } else if matching_tags.len() > 1 {
                    println!("Ambiguous match. Did you mean: {:?}", 
                        matching_tags);
                    continue;
                }

                // 2. If no match, treat as a new tag
                profile.learn(&item.entry.description, input);
                learned_any = true;
                save_profile_atomically(&profile_path, &profile)?;
                println!("Learned new tag: {}", input);
            }

            if learned_any {
                save_profile_atomically(&profile_path, &profile)?;
                println!("\nProfile updated and saved.");
            } else {
                println!("\nNo changes made to profile.");
            }
        }
        Commands::Tags { action } => match action {
            TagAction::Add { tag } => {
                let mut profile = Profile::load(&profile_path)
                    .map_err(|e| anyhow::anyhow!("Failed to load profile: {}", e))?;
                profile.add_tag(&tag);
                save_profile_atomically(&profile_path, &profile)?;
                println!("Added tag: {}", tag);
            }
            TagAction::Rm { tag } => {
                let mut profile = Profile::load(&profile_path)
                    .map_err(|e| anyhow::anyhow!("Failed to load profile: {}", e))?;
                profile.remove_tag(&tag);
                save_profile_atomically(&profile_path, &profile)?;
                println!("Removed tag: {}", tag);
            }
            TagAction::Category { tag, category } => {
                let mut profile = Profile::load(&profile_path)
                    .map_err(|e| anyhow::anyhow!("Failed to load profile: {}", e))?;
                profile.set_category(&tag, category.clone())
                    .map_err(|e| anyhow::anyhow!("Error setting category: {}", e))?;
                save_profile_atomically(&profile_path, &profile)?;
                println!("Assigned category {} to tag {}", category, tag);
            }
            TagAction::List => {
                let profile = Profile::load(&profile_path)
                    .map_err(|e| anyhow::anyhow!("Failed to load profile: {}", e))?;
                println!("{:<15} {:<15}", "Tag", "Category");
                println!("{:-<30}", "");
                for t in &profile.tags {
                    println!("{:<15} {:<15}", 
                        t.name, 
                        t.category.as_deref().unwrap_or("-")
                    );
                }
            }
        },
        Commands::Export { directory, output, summary } => {
            let dir = directory.as_ref().unwrap_or(&cli.directory);
            let profile = Profile::load(&profile_path)
                .map_err(|e| anyhow::anyhow!("Failed to load profile: {}", e))?;

            let imports = import_dir(dir, &profile)?;
            let result = profile.run(&imports);

            let mut csv_content = String::from("date,amount,description,tag\n");
            let mut category_totals = std::collections::HashMap::new();

            for (entry, sugg) in &result.tagged {
                let date_str = entry.date.map(|d| d.to_string()).unwrap_or_else(|| "unknown".to_string());
                let amount_val = entry.amount.unwrap_or(0.0);
                let amount_str = amount_val.to_string();
                
                csv_content.push_str(&format!("{},{},{},{}\n", date_str, amount_str, entry.description, sugg.tag));

                if let Some(cat) = profile.tags.iter().find(|t| t.name == sugg.tag).and_then(|t| t.category.as_ref()) {
                    *category_totals.entry(cat.clone()).or_insert(0.0) += amount_val;
                } else {
                    *category_totals.entry("uncategorized".to_string()).or_insert(0.0) += amount_val;
                }
            }

            fs::write(&output, csv_content)?;

            if summary {
                println!("\n--- Export Summary ---");
                println!("{:<20} {:<10}", "Category", "Total");
                println!("{:-<30}", "");
                let mut sorted_cats: Vec<_> = category_totals.into_iter().collect();
                sorted_cats.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
                for (cat, total) in sorted_cats {
                    println!("{:<20} {:<10.2}", cat, total);
                }
            }

            println!("Exported {} entries to {:?}", result.tagged.len(), output);
        }
    }

    Ok(())
}
