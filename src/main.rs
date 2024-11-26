//! `boxpop` extracts ("pops") files out of Docker containers ("boxes") and onto the local file system.

#![deny(clippy::unwrap_used)]
#![deny(unsafe_code)]
#![deny(missing_docs)]
#![warn(rust_2018_idioms)]

use boxpop::prelude::*;
use clap::{Parser, Subcommand};
use color_eyre::{eyre::Context, Result};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Layer};

pub mod cmd;

/// `boxpop` extracts ("pops") files out of Docker containers ("boxes") and onto the local file system.
#[derive(Debug, Parser)]
#[command(version, about)]
pub struct Application {
    /// The target image.
    #[clap(global = true)]
    image: Option<ImageRef>,

    /// The username to use when authenticating with the OCI registry.
    #[clap(global = true, long, requires = "password", env = "OCI_USERNAME")]
    username: Option<String>,

    /// The password to use when authenticating with the OCI registry.
    #[clap(global = true, long, requires = "username", env = "OCI_PASSWORD")]
    password: Option<String>,

    #[clap(subcommand)]
    command: Command,
}

/// Subcommands for the program.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Extracts the contents of the image to disk.
    ///
    /// By default:
    /// - Multiplatform images select the "most reasonable" platform based on where this program is running.
    /// - All image layers are "squished"; the exported files are the result of applying all layers in order.
    Extract(cmd::extract::Options),
}

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;
    init_tracing()?;

    let app = Application::parse();
    match app.command {
        Command::Extract(opts) => cmd::extract::main(opts).await?,
    }

    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    Ok(())
}

fn init_tracing() -> Result<()> {
    let filter = EnvFilter::builder()
        .with_default_directive("info".parse().context("parse built in directive")?)
        .from_env_lossy();

    tracing_subscriber::registry()
        .with(tracing_error::ErrorLayer::default())
        .with(
            tracing_subscriber::fmt::layer()
                .pretty()
                .with_filter(filter),
        )
        .try_init()
        .context("configure tracing")
}
