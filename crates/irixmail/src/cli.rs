use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "irixmail", version, about = "Self-hosted mail server")]
pub struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    Run,
    Setup,
    Admin {
        #[command(subcommand)]
        action: AdminAction,
    },
    Backup {
        path: PathBuf,
    },
    Restore {
        path: PathBuf,
    },
    Cert {
        #[command(subcommand)]
        action: CertAction,
    },
    Update {
        #[arg(long)]
        check: bool,
    },
}

#[derive(Subcommand)]
enum AdminAction {
    ResetPassword {
        email: String,
    },
    ApiKey {
        #[command(subcommand)]
        action: ApiKeyAction,
    },
}

#[derive(Subcommand)]
enum ApiKeyAction {
    Create { name: String },
    List,
    Revoke { id: String },
}

#[derive(Subcommand)]
enum CertAction {
    Status,
    Reissue,
}

pub fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command.unwrap_or(Command::Run) {
        Command::Run => crate::cmd_run::run()?,
        Command::Setup => crate::setup::run()?,
        Command::Admin { action } => match action {
            AdminAction::ResetPassword { email } => crate::cmd_reset_password::run(&email)?,
            AdminAction::ApiKey { action } => match action {
                ApiKeyAction::Create { name } => crate::cmd_api_key::create(&name)?,
                ApiKeyAction::List => crate::cmd_api_key::list()?,
                ApiKeyAction::Revoke { id } => crate::cmd_api_key::revoke(&id)?,
            },
        },
        Command::Backup { path } => crate::cmd_backup::run(&path)?,
        Command::Restore { path } => crate::cmd_restore::run(&path)?,
        Command::Cert { action } => match action {
            CertAction::Status => crate::cmd_cert_status::run()?,
            CertAction::Reissue => crate::cmd_cert_reissue::run()?,
        },
        Command::Update { check } => crate::cmd_update::run(check)?,
    }
    Ok(())
}
