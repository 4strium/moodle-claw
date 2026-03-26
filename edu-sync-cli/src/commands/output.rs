//! Output formatting utilities for agent-friendly output.

use serde::Serialize;

/// Output format for CLI commands.
#[derive(Debug, Clone, Copy, Default, clap::ValueEnum)]
pub enum OutputFormat {
    /// Human-readable markdown format (default).
    #[default]
    Markdown,
    /// Machine-readable JSON format.
    Json,
}

/// Trait for types that can be rendered in multiple formats.
pub trait Render {
    /// Render as markdown string.
    fn to_markdown(&self) -> String;
}

/// Print output in the specified format.
pub fn print_output<T: Serialize + Render>(data: &T, format: OutputFormat) {
    match format {
        OutputFormat::Markdown => {
            println!("{}", data.to_markdown());
        }
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(data).unwrap());
        }
    }
}

/// Print an error message.
pub fn print_error(error: &str, format: OutputFormat) {
    match format {
        OutputFormat::Markdown => {
            eprintln!("Error: {}", error);
        }
        OutputFormat::Json => {
            eprintln!(r#"{{"error": "{}"}}"#, error.replace('"', r#"\""#));
        }
    }
}
