//! Display connection status.

use edu_sync::config::Config;
use serde::Serialize;

use super::output::{print_error, print_output, OutputFormat, Render};

/// Display current configuration and connection status.
#[derive(Debug, clap::Parser)]
pub struct Command {
    /// Output format.
    #[arg(long, short, value_enum, default_value_t = OutputFormat::Markdown)]
    output: OutputFormat,
}

#[derive(Debug, Serialize)]
struct AccountStatus {
    id: String,
    user: String,
    site: String,
    site_url: String,
    download_path: String,
    total_courses: usize,
    synced_courses: usize,
}

#[derive(Debug, Serialize)]
struct StatusResult {
    configured: bool,
    config_path: String,
    accounts: Vec<AccountStatus>,
}

impl Render for StatusResult {
    fn to_markdown(&self) -> String {
        if !self.configured {
            return format!(
                "**Not configured**\n\n\
                 Run `moodle configure` to set up a Moodle connection.\n\n\
                 Config file: `{}`",
                self.config_path
            );
        }

        let mut output = format!(
            "## Moodle Status\n\n**Config file**: `{}`\n\n",
            self.config_path
        );

        if self.accounts.is_empty() {
            output.push_str("No accounts configured.\n");
        } else {
            output.push_str("### Accounts\n\n");
            for account in &self.accounts {
                output.push_str(&format!(
                    "#### {}\n\
                     - **User**: {}\n\
                     - **Site**: {}\n\
                     - **URL**: {}\n\
                     - **Download path**: `{}`\n\
                     - **Courses**: {} total, {} synced\n\n",
                    account.id,
                    account.user,
                    account.site,
                    account.site_url,
                    account.download_path,
                    account.total_courses,
                    account.synced_courses
                ));
            }
        }

        output
    }
}

impl Command {
    pub async fn run(self) -> anyhow::Result<()> {
        let config_path = Config::path().display().to_string();

        let config = match Config::read().await {
            Ok(c) => c,
            Err(e) => {
                print_error(&format!("Failed to read config: {}", e), self.output);
                return Ok(());
            }
        };

        let accounts: Vec<AccountStatus> = config
            .accounts
            .iter()
            .map(|(id, account)| {
                let total_courses = account.courses.0.len();
                let synced_courses = account.courses.0.values().filter(|c| c.sync).count();
                AccountStatus {
                    id: id.clone(),
                    user: account.user.clone(),
                    site: account.site.clone(),
                    site_url: account.id.site_url.to_string(),
                    download_path: account.path.display().to_string(),
                    total_courses,
                    synced_courses,
                }
            })
            .collect();

        let result = StatusResult {
            configured: config.has_accounts(),
            config_path,
            accounts,
        };

        print_output(&result, self.output);

        Ok(())
    }
}
