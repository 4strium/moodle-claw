//! Download and retrieve a specific file.

use std::path::PathBuf;

use edu_sync::{config::Config, util::sanitize_path_component};
use edu_ws::token::Token;
use serde::Serialize;
use tokio::{
    fs::{self, File},
    io::AsyncWriteExt,
};

use super::{
    courses::find_course,
    output::{print_error, print_output, OutputFormat, Render},
};

/// Download and return the content or path of a file from Moodle.
#[derive(Debug, clap::Parser)]
pub struct Command {
    /// File path within course structure (e.g., "Course/Section/Module/file.pdf").
    /// Or use --url for direct URL download.
    #[arg(required_unless_present = "url")]
    path: Option<String>,
    /// Direct file URL from Moodle.
    #[arg(long, conflicts_with = "path")]
    url: Option<String>,
    /// Course name or ID (required when using path without full course name).
    #[arg(long, short)]
    course: Option<String>,
    /// Output directory (defaults to cache directory).
    #[arg(long, short = 'd')]
    dest: Option<PathBuf>,
    /// Output format.
    #[arg(long, short, value_enum, default_value_t = OutputFormat::Markdown)]
    output: OutputFormat,
}

#[derive(Debug, Serialize)]
struct GetResult {
    status: String,
    file_name: String,
    local_path: String,
    size: u64,
}

impl Render for GetResult {
    fn to_markdown(&self) -> String {
        format!(
            "Downloaded: `{}`\n\
             Local path: `{}`\n\
             Size: {}",
            self.file_name,
            self.local_path,
            format_size(self.size)
        )
    }
}

fn format_size(size: u64) -> String {
    if size >= 1024 * 1024 {
        format!("{:.1} MB", size as f64 / (1024.0 * 1024.0))
    } else if size >= 1024 {
        format!("{:.1} KB", size as f64 / 1024.0)
    } else {
        format!("{} B", size)
    }
}

/// Get the cache directory for downloaded files.
fn cache_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("moodle-claw")
        .join("files")
}

impl Command {
    pub async fn run(self) -> anyhow::Result<()> {
        let config = Config::read().await?;

        if !config.has_accounts() {
            print_error(
                "No accounts configured. Run `moodle configure` first.",
                self.output,
            );
            return Ok(());
        }

        // Determine download directory
        let dest_dir = self.dest.clone().unwrap_or_else(cache_dir);
        fs::create_dir_all(&dest_dir).await?;

        // Handle direct URL download
        if let Some(ref url_str) = self.url {
            let url: reqwest::Url = url_str.parse()?;

            // Find a token to use (use first account)
            let (_, account_config) = config.accounts.iter().next().ok_or_else(|| {
                anyhow::anyhow!("No account configured")
            })?;

            return self
                .download_url(&url, &account_config.token, &dest_dir)
                .await;
        }

        // Handle path-based download
        let file_path = self.path.as_ref().unwrap();
        let parts: Vec<&str> = file_path.split('/').collect();

        if parts.len() < 2 {
            print_error(
                "Path must include at least section/file or module/file",
                self.output,
            );
            return Ok(());
        }

        // Find the course
        let course_query = self.course.as_deref().unwrap_or(parts[0]);
        let (course_id, course_name, account_config) = match find_course(&config, course_query) {
            Some(c) => c,
            None => {
                print_error(
                    &format!(
                        "Course '{}' not found. Run `moodle courses --refresh` first.",
                        course_query
                    ),
                    self.output,
                );
                return Ok(());
            }
        };

        // Fetch course content to find the file
        let ws_client = edu_ws::ws::Client::new(
            edu_sync::util::shared_http(),
            &account_config.id.site_url,
            account_config.token,
            account_config.id.lang.clone(),
        );

        let sections = match ws_client.get_contents(course_id).await {
            Ok(s) => s,
            Err(e) => {
                print_error(&format!("Failed to fetch course content: {}", e), self.output);
                return Ok(());
            }
        };

        // Search for the file
        let search_name = parts.last().unwrap().to_lowercase();
        let search_parts: Vec<String> = parts.iter().map(|p| p.to_lowercase()).collect();

        for section in &sections {
            let section_matches = search_parts
                .iter()
                .any(|p| section.name.to_lowercase().contains(p));

            for module in &section.modules {
                let module_matches = search_parts
                    .iter()
                    .any(|p| module.name.to_lowercase().contains(p));

                if let Some(contents) = &module.contents {
                    for content in contents {
                        let file_matches = content.name.to_lowercase().contains(&search_name)
                            || content.name.to_lowercase() == search_name;

                        if file_matches && (section_matches || module_matches || parts.len() == 1) {
                            if let Some(url) = &content.url {
                                // Create subdirectory structure
                                let sanitized_course = sanitize_path_component(course_name);
                                let sanitized_section = sanitize_path_component(&section.name);
                                let sanitized_module = sanitize_path_component(&module.name);

                                let file_dir = dest_dir
                                    .join(sanitized_course.as_ref())
                                    .join(sanitized_section.as_ref())
                                    .join(sanitized_module.as_ref());

                                fs::create_dir_all(&file_dir).await?;

                                let file_path = file_dir.join(&content.name);

                                // Check if already cached
                                if file_path.exists() {
                                    let metadata = fs::metadata(&file_path).await?;
                                    let result = GetResult {
                                        status: "cached".to_string(),
                                        file_name: content.name.clone(),
                                        local_path: file_path.display().to_string(),
                                        size: metadata.len(),
                                    };
                                    print_output(&result, self.output);
                                    return Ok(());
                                }

                                // Download the file
                                let mut download_url = url.clone();
                                account_config.token.apply(&mut download_url);

                                let response = edu_sync::util::shared_http()
                                    .get(download_url)
                                    .send()
                                    .await?;

                                let bytes = response.bytes().await?;
                                let size = bytes.len() as u64;

                                let mut file = File::create(&file_path).await?;
                                file.write_all(&bytes).await?;
                                file.flush().await?;

                                let result = GetResult {
                                    status: "downloaded".to_string(),
                                    file_name: content.name.clone(),
                                    local_path: file_path.display().to_string(),
                                    size,
                                };

                                print_output(&result, self.output);
                                return Ok(());
                            }
                        }
                    }
                }
            }
        }

        print_error(
            &format!("File '{}' not found in course '{}'", file_path, course_name),
            self.output,
        );

        Ok(())
    }

    async fn download_url(
        &self,
        url: &reqwest::Url,
        token: &Token,
        dest_dir: &PathBuf,
    ) -> anyhow::Result<()> {
        let mut download_url = url.clone();
        token.apply(&mut download_url);

        // Extract filename from URL
        let file_name = url
            .path_segments()
            .and_then(|segments| segments.last())
            .unwrap_or("downloaded_file")
            .to_string();

        let file_path = dest_dir.join(&file_name);

        let response = edu_sync::util::shared_http()
            .get(download_url)
            .send()
            .await?;

        let bytes = response.bytes().await?;
        let size = bytes.len() as u64;

        let mut file = File::create(&file_path).await?;
        file.write_all(&bytes).await?;
        file.flush().await?;

        let result = GetResult {
            status: "downloaded".to_string(),
            file_name,
            local_path: file_path.display().to_string(),
            size,
        };

        print_output(&result, self.output);

        Ok(())
    }
}
