//! Search for content across courses.

use edu_sync::config::Config;
use serde::Serialize;

use super::{
    courses::find_course,
    output::{print_error, print_output, OutputFormat, Render},
};

/// Search for content across courses.
#[derive(Debug, clap::Parser)]
pub struct Command {
    /// Search query (case-insensitive substring match).
    query: String,
    /// Limit search to a specific course (name or ID).
    #[arg(long, short)]
    course: Option<String>,
    /// Filter by content type (file, url, folder).
    #[arg(long, short = 't')]
    content_type: Option<String>,
    /// Output format.
    #[arg(long, short, value_enum, default_value_t = OutputFormat::Markdown)]
    output: OutputFormat,
}

#[derive(Debug, Serialize, Clone)]
struct SearchResult {
    course_id: u64,
    course_name: String,
    section: String,
    module: String,
    name: String,
    #[serde(rename = "type")]
    content_type: String,
    path: String,
    size: u64,
    url: Option<String>,
}

#[derive(Debug, Serialize)]
struct SearchResults {
    query: String,
    total: usize,
    results: Vec<SearchResult>,
}

impl Render for SearchResults {
    fn to_markdown(&self) -> String {
        if self.results.is_empty() {
            return format!("No results found for '{}'.\n", self.query);
        }

        let mut output = format!(
            "## Search Results for '{}' ({} found)\n\n",
            self.query, self.total
        );

        let mut current_course: Option<&str> = None;
        for result in &self.results {
            if current_course != Some(&result.course_name) {
                output.push_str(&format!("### {}\n\n", result.course_name));
                current_course = Some(&result.course_name);
            }

            let size_str = if result.size > 0 {
                format!(" [{}]", format_size(result.size))
            } else {
                String::new()
            };

            output.push_str(&format!(
                "- **{}** > **{}** > `{}`{}\n",
                result.section, result.module, result.name, size_str
            ));
        }

        output
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

        let query_lower = self.query.to_lowercase();
        let mut results: Vec<SearchResult> = Vec::new();

        // Determine which courses to search
        let courses_to_search: Vec<(u64, String, &edu_sync::config::AccountConfig)> =
            if let Some(ref course_query) = self.course {
                match find_course(&config, course_query) {
                    Some((id, name, account)) => vec![(id, name.to_string(), account)],
                    None => {
                        print_error(
                            &format!("Course '{}' not found.", course_query),
                            self.output,
                        );
                        return Ok(());
                    }
                }
            } else {
                // Search all courses
                config
                    .accounts
                    .values()
                    .flat_map(|account| {
                        account
                            .courses
                            .0
                            .iter()
                            .map(move |(id, course)| (*id, course.name.clone(), account))
                    })
                    .collect()
            };

        // Search each course
        for (course_id, course_name, account_config) in courses_to_search {
            let ws_client = edu_ws::ws::Client::new(
                edu_sync::util::shared_http(),
                &account_config.id.site_url,
                account_config.token,
                account_config.id.lang.clone(),
            );

            let sections = match ws_client.get_contents(course_id).await {
                Ok(s) => s,
                Err(_) => continue, // Skip this course on error
            };

            for section in sections {
                for module in section.modules {
                    // Check module name
                    let module_matches = module.name.to_lowercase().contains(&query_lower);

                    if let Some(contents) = module.contents {
                        for content in contents {
                            let content_matches =
                                content.name.to_lowercase().contains(&query_lower);

                            if module_matches || content_matches {
                                // Apply type filter if specified
                                if let Some(ref type_filter) = self.content_type {
                                    let type_str = format!("{:?}", content.ty).to_lowercase();
                                    if !type_str.contains(&type_filter.to_lowercase()) {
                                        continue;
                                    }
                                }

                                results.push(SearchResult {
                                    course_id,
                                    course_name: course_name.clone(),
                                    section: section.name.clone(),
                                    module: module.name.clone(),
                                    name: content.name.clone(),
                                    content_type: format!("{:?}", content.ty),
                                    path: format!(
                                        "{}/{}/{}/{}",
                                        course_name, section.name, module.name, content.name
                                    ),
                                    size: content.size,
                                    url: content.url.map(|u| u.to_string()),
                                });
                            }
                        }
                    }
                }
            }
        }

        let search_results = SearchResults {
            query: self.query,
            total: results.len(),
            results,
        };

        print_output(&search_results, self.output);

        Ok(())
    }
}
