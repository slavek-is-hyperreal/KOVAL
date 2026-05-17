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
    }
}
