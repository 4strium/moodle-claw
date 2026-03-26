//! List available courses.

use edu_sync::{account::Account, config::Config};
use serde::Serialize;

use super::output::{print_error, print_output, OutputFormat, Render};

/// List all courses the user is enrolled in.
#[derive(Debug, clap::Parser)]
pub struct Command {
    /// Filter courses by name (case-insensitive substring match).
    #[arg(long, short)]
    filter: Option<String>,
    /// Include hidden courses.
    #[arg(long)]
    include_hidden: bool,
    /// Refresh course list from server (don't use cached data).
    #[arg(long, short)]
    refresh: bool,
    /// Output format.
    #[arg(long, short, value_enum, default_value_t = OutputFormat::Markdown)]
    output: OutputFormat,
}

#[derive(Debug, Serialize, Clone)]
struct CourseInfo {
    id: u64,
    name: String,
    short_name: String,
    visible: bool,
    sync_enabled: bool,
    last_access: Option<String>,
    account: String,
}

#[derive(Debug, Serialize)]
struct CoursesResult {
    total: usize,
    courses: Vec<CourseInfo>,
}

impl Render for CoursesResult {
    fn to_markdown(&self) -> String {
        if self.courses.is_empty() {
            return "No courses found.\n\nRun `moodle courses --refresh` to fetch courses from the server.".to_string();
        }

        let mut output = format!("## Your Courses ({} total)\n\n", self.total);
        output.push_str("| ID | Name | Sync | Last Access |\n");
        output.push_str("|----|------|------|-------------|\n");

        for course in &self.courses {
            let sync_status = if course.sync_enabled { "yes" } else { "no" };
            let last_access = course.last_access.as_deref().unwrap_or("N/A");
            output.push_str(&format!(
                "| {} | {} | {} | {} |\n",
                course.id, course.name, sync_status, last_access
            ));
        }

        output
    }
}

impl Command {
    pub async fn run(self) -> anyhow::Result<()> {
        let mut config = Config::read().await?;

        if !config.has_accounts() {
            print_error(
                "No accounts configured. Run `moodle configure` first.",
                self.output,
            );
            return Ok(());
        }

        // If refresh requested or no courses cached, fetch from server
        if self.refresh || !config.has_courses() {
            for (account_id, account_config) in config.accounts.iter_mut() {
                let account = Account::new(account_config.id.clone(), account_config.token);
                match account.get_courses().await {
                    Ok(courses) => {
                        account_config.courses.update(courses);
                    }
                    Err(e) => {
                        print_error(
                            &format!("Failed to fetch courses for {}: {}", account_id, e),
                            self.output,
                        );
                    }
                }
            }
            config.write().await?;
        }

        // Build course list
        let mut courses: Vec<CourseInfo> = Vec::new();
        for (account_id, account_config) in &config.accounts {
            for (course_id, course_config) in &account_config.courses.0 {
                let course_info = CourseInfo {
                    id: *course_id,
                    name: course_config.name.clone(),
                    short_name: course_config.name.clone(), // Simplified
                    visible: true,
                    sync_enabled: course_config.sync,
                    last_access: None, // Not stored in config currently
                    account: account_id.clone(),
                };
                courses.push(course_info);
            }
        }

        // Apply filter
        if let Some(ref filter) = self.filter {
            let filter_lower = filter.to_lowercase();
            courses.retain(|c| c.name.to_lowercase().contains(&filter_lower));
        }

        // Filter hidden if necessary
        if !self.include_hidden {
            courses.retain(|c| c.visible);
        }

        // Sort by name
        courses.sort_by(|a, b| a.name.cmp(&b.name));

        let result = CoursesResult {
            total: courses.len(),
            courses,
        };

        print_output(&result, self.output);

        Ok(())
    }
}

/// Fuzzy match a course by name or ID.
pub fn find_course<'a>(
    config: &'a Config,
    query: &str,
) -> Option<(u64, &'a str, &'a edu_sync::config::AccountConfig)> {
    // Try parsing as ID first
    if let Ok(id) = query.parse::<u64>() {
        for account in config.accounts.values() {
            if let Some(course) = account.courses.0.get(&id) {
                return Some((id, &course.name, account));
            }
        }
    }

    // Fuzzy match by name
    let query_lower = query.to_lowercase();
    let mut best_match: Option<(u64, &str, &edu_sync::config::AccountConfig, usize)> = None;

    for account in config.accounts.values() {
        for (id, course) in &account.courses.0 {
            let name_lower = course.name.to_lowercase();

            // Exact match
            if name_lower == query_lower {
                return Some((*id, &course.name, account));
            }

            // Contains match (prefer shorter matches)
            if name_lower.contains(&query_lower) {
                let score = name_lower.len();
                if best_match.is_none() || score < best_match.as_ref().unwrap().3 {
                    best_match = Some((*id, &course.name, account, score));
                }
            }
        }
    }

    best_match.map(|(id, name, account, _)| (id, name, account))
}
