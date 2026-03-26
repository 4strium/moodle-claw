//! Moodle CLI - A command-line interface for interacting with Moodle LMS.
//!
//! This tool allows you to configure Moodle connections, list courses,
//! browse course content, search for files, download resources, and
//! synchronize courses to local storage.

#![warn(rust_2018_idioms)]
#![warn(clippy::default_trait_access)]
#![warn(clippy::inconsistent_struct_constructor)]
#![warn(clippy::semicolon_if_nothing_returned)]
#![deny(rustdoc::all)]

mod commands;

use std::env;

use clap::{CommandFactory, Parser};
use human_panic::setup_panic;
use tracing_subscriber::EnvFilter;

/// Moodle command-line interface for working with Moodle LMS.
#[derive(Debug, Parser)]
#[command(name = "moodle", author, version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, clap::Subcommand)]
enum Command {
    /// Configure connection to a Moodle server.
    Configure(commands::configure::Command),
    /// Display current configuration and connection status.
    Status(commands::status::Command),
    /// List all courses the user is enrolled in.
    Courses(commands::courses::Command),
    /// Get the structure and content of a specific course.
    Content(commands::content::Command),
    /// Search for content across courses.
    Search(commands::search::Command),
    /// Download and return a file from Moodle.
    Get(commands::get::Command),
    /// Synchronize content from courses to local storage.
    Sync(commands::sync::Command),
}

impl Cli {
    async fn run(self) -> anyhow::Result<()> {
        match self.command {
            Command::Configure(cmd) => cmd.run().await,
            Command::Status(cmd) => cmd.run().await,
            Command::Courses(cmd) => cmd.run().await,
            Command::Content(cmd) => cmd.run().await,
            Command::Search(cmd) => cmd.run().await,
            Command::Get(cmd) => cmd.run().await,
            Command::Sync(cmd) => cmd.run().await,
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Enable shell completions
    clap_complete::CompleteEnv::with_factory(Cli::command).complete();

    // Set up logging
    let fmt = tracing_subscriber::fmt().with_writer(std::io::stderr);
    if env::var_os(EnvFilter::DEFAULT_ENV).is_some() {
        fmt.with_env_filter(EnvFilter::try_from_default_env()?)
            .init();
    } else {
        fmt.init();
    }

    // Set up panic handler
    setup_panic!();

    // Run the CLI
    Cli::parse().run().await
}
