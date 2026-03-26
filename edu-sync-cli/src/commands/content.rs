//! Display course content structure.

use edu_sync::config::Config;
use serde::Serialize;

use super::{
    courses::find_course,
    output::{print_error, print_output, OutputFormat, Render},
};

/// Get the structure and content of a specific course.
#[derive(Debug, clap::Parser)]
pub struct Command {
    /// Course name or ID.
    course: String,
    /// Filter to a specific section (case-insensitive substring match).
    #[arg(long, short)]
    section: Option<String>,
    /// Output format.
    #[arg(long, short, value_enum, default_value_t = OutputFormat::Markdown)]
    output: OutputFormat,
}

#[derive(Debug, Serialize, Clone)]
struct FileInfo {
    name: String,
    #[serde(rename = "type")]
    file_type: String,
    size: u64,
    path: String,
    url: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
struct ModuleInfo {
    id: u64,
    name: String,
    #[serde(rename = "type")]
    module_type: String,
    files: Vec<FileInfo>,
}

#[derive(Debug, Serialize, Clone)]
struct SectionInfo {
    id: i64,
    name: String,
    summary: String,
    modules: Vec<ModuleInfo>,
}

#[derive(Debug, Serialize)]
struct ContentResult {
    course_id: u64,
    course_name: String,
    sections: Vec<SectionInfo>,
}

impl Render for ContentResult {
    fn to_markdown(&self) -> String {
        let mut output = format!("## {} (ID: {})\n\n", self.course_name, self.course_id);

        if self.sections.is_empty() {
            output.push_str("No content found.\n");
            return output;
        }

        for section in &self.sections {
            output.push_str(&format!("### {}\n\n", section.name));

            if !section.summary.is_empty() {
                // Strip HTML tags from summary for cleaner output
                let clean_summary = strip_html_tags(&section.summary);
                if !clean_summary.trim().is_empty() {
                    output.push_str(&format!("{}\n\n", clean_summary.trim()));
                }
            }

            if section.modules.is_empty() {
                output.push_str("*No content*\n\n");
                continue;
            }

            for module in &section.modules {
                output.push_str(&format!("- **{}** ({})\n", module.name, module.module_type));

                for file in &module.files {
                    let size_str = if file.size > 0 {
                        format_size(file.size)
                    } else {
                        "".to_string()
                    };
                    let size_suffix = if size_str.is_empty() {
                        "".to_string()
                    } else {
                        format!(" [{}]", size_str)
                    };
                    output.push_str(&format!("  - `{}`{}\n", file.name, size_suffix));
                }
            }
            output.push('\n');
        }

        output
    }
}

fn strip_html_tags(html: &str) -> String {
    let mut result = String::new();
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => result.push(c),
            _ => {}
        }
    }
    result
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

        // Find the course
        let (course_id, course_name, account_config) = match find_course(&config, &self.course) {
            Some(c) => c,
            None => {
                print_error(
                    &format!("Course '{}' not found. Run `moodle courses --refresh` to update the course list.", self.course),
                    self.output,
                );
                return Ok(());
            }
        };

        // Fetch content from Moodle
        let ws_client = edu_ws::ws::Client::new(
            edu_sync::util::shared_http(),
            &account_config.id.site_url,
            account_config.token,
            account_config.id.lang.clone(),
        );

        let sections = match ws_client.get_contents(course_id).await {
            Ok(s) => s,
            Err(e) => {
                print_error(
                    &format!("Failed to fetch course content: {}", e),
                    self.output,
                );
                return Ok(());
            }
        };

        // Transform to our structure
        let mut section_infos: Vec<SectionInfo> = sections
            .into_iter()
            .map(|section| {
                let modules: Vec<ModuleInfo> = section
                    .modules
                    .into_iter()
                    .map(|module| {
                        let files: Vec<FileInfo> = module
                            .contents
                            .unwrap_or_default()
                            .into_iter()
                            .map(|content| FileInfo {
                                name: content.name.clone(),
                                file_type: format!("{:?}", content.ty),
                                size: content.size,
                                path: content
                                    .path
                                    .map(|p| p.display().to_string())
                                    .unwrap_or_default(),
                                url: content.url.map(|u| u.to_string()),
                            })
                            .collect();

                        ModuleInfo {
                            id: module.id,
                            name: module.name,
                            module_type: module.ty,
                            files,
                        }
                    })
                    .collect();

                SectionInfo {
                    id: section.id,
                    name: section.name,
                    summary: section.summary,
                    modules,
                }
            })
            .collect();

        // Apply section filter
        if let Some(ref filter) = self.section {
            let filter_lower = filter.to_lowercase();
            section_infos.retain(|s| s.name.to_lowercase().contains(&filter_lower));
        }

        let result = ContentResult {
            course_id,
            course_name: course_name.to_string(),
            sections: section_infos,
        };

        print_output(&result, self.output);

        Ok(())
    }
}
