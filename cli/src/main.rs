use clap::{Args, Parser, Subcommand};
use crate::config::Config;
use crate::client::ApiClient;

pub mod config;
pub mod client;
pub mod output;

#[derive(Parser, Debug)]
#[command(name = "koval", version = "0.1.0", about = "Koval High-Performance Compiler Sandbox CLI")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Manage CLI configuration settings
    Config(ConfigArgs),
    /// Administrative Developer Token management
    Token(TokenArgs),
    /// Track and inspect compilation job histories
    Job(JobArgs),
    /// Manage active webhook notification endpoints
    Webhook(WebhookArgs),
    /// Profile-Guided Optimization (PGO) operations
    Pgo(PgoArgs),
}

#[derive(Args, Debug)]
struct ConfigArgs {
    #[command(subcommand)]
    command: ConfigSubcommands,
}

#[derive(Subcommand, Debug)]
enum ConfigSubcommands {
    /// Save the target Koval server API base URL
    SetServer {
        url: String,
    },
    /// Save the developer/admin Bearer token
    SetToken {
        token: String,
    },
    /// View the current active CLI configuration
    Show,
}

#[derive(Args, Debug)]
struct TokenArgs {
    #[command(subcommand)]
    command: TokenSubcommands,
}

#[derive(Subcommand, Debug)]
enum TokenSubcommands {
    /// Create a new developer Bearer token
    Create {
        #[arg(long)]
        name: String,
    },
    /// List all currently active tokens (Admin only)
    List,
    /// Revoke/Deactivate a token by ID
    Delete {
        id: i64,
    },
}

#[derive(Args, Debug)]
struct JobArgs {
    #[command(subcommand)]
    command: JobSubcommands,
}

#[derive(Subcommand, Debug)]
enum JobSubcommands {
    /// List last 50 jobs for the active developer token
    List,
    /// Inspect full, raw detailed status JSON for a specific job
    Status {
        job_id: String,
    },
}

#[derive(Args, Debug)]
struct WebhookArgs {
    #[command(subcommand)]
    command: WebhookSubcommands,
}

#[derive(Subcommand, Debug)]
enum WebhookSubcommands {
    /// Register a new webhook target endpoint with HMAC signing secret
    Create {
        #[arg(long)]
        url: String,
        #[arg(long)]
        secret: String,
    },
    /// List registered webhooks for the active token
    List,
    /// Deactivate a webhook endpoint by ID
    Delete {
        id: i64,
    },
}

#[derive(Args, Debug)]
struct PgoArgs {
    #[command(subcommand)]
    command: PgoSubcommands,
}

#[derive(Subcommand, Debug)]
enum PgoSubcommands {
    /// Submit a new job with PGO phase set to 'instrument'
    Instrument {
        /// Project git URL
        project: String,
        /// Git reference (branch, tag, or commit SHA)
        #[arg(long, default_value = "main")]
        git_ref: String,
        /// target architecture target triple
        #[arg(long)]
        target: Option<String>,
        /// cpu profile flag (e.g. native, x86-64-v3)
        #[arg(long, default_value = "native")]
        cpu: String,
    },
    /// Upload profraw files and trigger optimization phase
    Upload {
        /// Instrumented job ID
        instrument_job_id: String,
        /// Directory containing .profraw files
        profiles_dir: String,
    },
}

fn main() {
    let args = Cli::parse();
    let mut config = Config::load();

    match args.command {
        Commands::Config(cfg_args) => match cfg_args.command {
            ConfigSubcommands::SetServer { url } => {
                config.server_url = Some(url.clone());
                match config.save() {
                    Ok(_) => println!("Server URL successfully saved: {}", url),
                    Err(e) => eprintln!("Error: {}", e),
                }
            }
            ConfigSubcommands::SetToken { token } => {
                config.token = Some(token);
                match config.save() {
                    Ok(_) => println!("Bearer token successfully saved."),
                    Err(e) => eprintln!("Error: {}", e),
                }
            }
            ConfigSubcommands::Show => {
                println!("Config file: {:?}", Config::file_path());
                println!("Server URL:  {}", config.server_url.as_deref().unwrap_or("<not set>"));
                println!("Token:       {}", config.token.as_ref().map(|_| "********").unwrap_or("<not set>"));
            }
        },
        Commands::Token(tok_args) => {
            let client = match ApiClient::new(&config) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("Error: {}", e);
                    std::process::exit(1);
                }
            };
            match tok_args.command {
                TokenSubcommands::Create { name } => {
                    match client.create_token(&name) {
                        Ok(res) => {
                            println!("=======================================================");
                            println!("  TOKEN CREATED SUCCESSFULLY");
                            println!("  Name:      {}", res.name);
                            println!("  ID:        {}", res.id);
                            println!("  Token:     {}", res.plaintext_token);
                            println!("=======================================================");
                            println!("  WARNING: Copy this token immediately. It will NOT");
                            println!("  be shown again and cannot be retrieved!");
                            println!("=======================================================");
                        }
                        Err(e) => {
                            eprintln!("Failed to create token: {}", e);
                            std::process::exit(1);
                        }
                    }
                }
                TokenSubcommands::List => {
                    match client.list_tokens() {
                        Ok(list) => output::print_tokens(&list),
                        Err(e) => {
                            eprintln!("Failed to list active tokens: {}", e);
                            std::process::exit(1);
                        }
                    }
                }
                TokenSubcommands::Delete { id } => {
                    match client.delete_token(id) {
                        Ok(_) => println!("Token successfully revoked/deactivated."),
                        Err(e) => {
                            eprintln!("Failed to delete token: {}", e);
                            std::process::exit(1);
                        }
                    }
                }
            }
        }
        Commands::Job(job_args) => {
            let client = match ApiClient::new(&config) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("Error: {}", e);
                    std::process::exit(1);
                }
            };
            match job_args.command {
                JobSubcommands::List => {
                    match client.list_jobs() {
                        Ok(list) => output::print_jobs(&list),
                        Err(e) => {
                            eprintln!("Failed to list jobs: {}", e);
                            std::process::exit(1);
                        }
                    }
                }
                JobSubcommands::Status { job_id } => {
                    match client.job_status(&job_id) {
                        Ok(json) => {
                            println!("{}", serde_json::to_string_pretty(&json).unwrap_or_default());
                        }
                        Err(e) => {
                            eprintln!("Failed to fetch status for job {}: {}", job_id, e);
                            std::process::exit(1);
                        }
                    }
                }
            }
        }
        Commands::Webhook(wh_args) => {
            let client = match ApiClient::new(&config) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("Error: {}", e);
                    std::process::exit(1);
                }
            };
            match wh_args.command {
                WebhookSubcommands::Create { url, secret } => {
                    match client.create_webhook(&url, &secret) {
                        Ok(_) => println!("Webhook registered successfully."),
                        Err(e) => {
                            eprintln!("Failed to register webhook: {}", e);
                            std::process::exit(1);
                        }
                    }
                }
                WebhookSubcommands::List => {
                    match client.list_webhooks() {
                        Ok(list) => output::print_webhooks(&list),
                        Err(e) => {
                            eprintln!("Failed to list webhooks: {}", e);
                            std::process::exit(1);
                        }
                    }
                }
                WebhookSubcommands::Delete { id } => {
                    match client.delete_webhook(id) {
                        Ok(_) => println!("Webhook successfully deleted/deactivated."),
                        Err(e) => {
                            eprintln!("Failed to delete webhook: {}", e);
                            std::process::exit(1);
                        }
                    }
                }
            }
        }
        Commands::Pgo(pgo_args) => {
            let client = match ApiClient::new(&config) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("Error: {}", e);
                    std::process::exit(1);
                }
            };
            match pgo_args.command {
                PgoSubcommands::Instrument { project, git_ref, target, cpu } => {
                    let hardware = schema::HardwareProfile {
                        cpu: schema::CpuProfile {
                            flags: vec![cpu],
                            ..Default::default()
                        },
                        ..Default::default()
                    };
                    let req = schema::JobRequest {
                        hardware,
                        project,
                        git_ref,
                        binary: None,
                        package: None,
                        target,
                        pgo_phase: Some("instrument".to_string()),
                    };
                    match client.submit_job(&req) {
                        Ok(res) => {
                            let job_id = res.get("id").and_then(|id| id.as_str()).unwrap_or("unknown");
                            println!("=======================================================");
                            println!("  PGO INSTRUMENTATION JOB SUBMITTED SUCCESSFULLY");
                            println!("  Job ID: {}", job_id);
                            println!("=======================================================");
                        }
                        Err(e) => {
                            eprintln!("Failed to submit instrumentation job: {}", e);
                            std::process::exit(1);
                        }
                    }
                }
                PgoSubcommands::Upload { instrument_job_id, profiles_dir } => {
                    let dir_path = std::path::Path::new(&profiles_dir);
                    if !dir_path.exists() || !dir_path.is_dir() {
                        eprintln!("Error: Directory '{}' does not exist or is not a directory", profiles_dir);
                        std::process::exit(1);
                    }
                    
                    let entries = match std::fs::read_dir(dir_path) {
                        Ok(e) => e,
                        Err(err) => {
                            eprintln!("Failed to read directory '{}': {}", profiles_dir, err);
                            std::process::exit(1);
                        }
                    };

                    let mut files = Vec::new();
                    for entry in entries {
                        let entry = match entry {
                            Ok(e) => e,
                            Err(_) => continue,
                        };
                        let path = entry.path();
                        if path.is_file() {
                            if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
                                if ext == "profraw" {
                                    let filename = path.file_name().and_then(|s| s.to_str()).unwrap_or("").to_string();
                                    let content = match std::fs::read(&path) {
                                        Ok(c) => c,
                                        Err(err) => {
                                            eprintln!("Failed to read file '{}': {}", path.display(), err);
                                            std::process::exit(1);
                                        }
                                    };
                                    files.push((filename, content));
                                }
                            }
                        }
                    }

                    if files.is_empty() {
                        eprintln!("Error: No .profraw files found in directory '{}'", profiles_dir);
                        std::process::exit(1);
                    }

                    println!("Uploading {} .profraw files...", files.len());
                    match client.upload_pgo_profiles(&instrument_job_id, files) {
                        Ok(res) => {
                            println!("=======================================================");
                            println!("  PGO PROFILES UPLOADED & MERGED SUCCESSFULLY");
                            println!("  Merged Profile URL:   {}", res.merged_profile_url);
                            println!("  Optimization Job ID:  {}", res.optimization_job_id);
                            println!("=======================================================");
                        }
                        Err(e) => {
                            eprintln!("Failed to upload profiles: {}", e);
                            std::process::exit(1);
                        }
                    }
                }
            }
        }
    }
}
