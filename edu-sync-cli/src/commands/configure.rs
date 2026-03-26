//! Configure Moodle server connection.

use std::path::PathBuf;

use base64::prelude::*;
use dialoguer::{Input, Password, Select};
use edu_sync::{
    account::Account,
    config::{self, AccountConfig, Config},
};
use edu_ws::token::Token;
use serde::Serialize;
use tokio::task;
use url::Url;

use super::output::{print_error, print_output, OutputFormat, Render};

/// Configure connection to a Moodle server.
#[derive(Debug, clap::Parser)]
pub struct Command {
    /// Moodle server URL.
    #[arg(long)]
    url: Option<Url>,
    /// Authentication token (32-character hex string).
    #[arg(long, conflicts_with_all = ["username", "password", "sso_url"])]
    token: Option<String>,
    /// SSO redirect URL (moodledl://token=... or moodlemobile://token=...).
    /// Use this for SSO authentication - paste the full redirect URL here.
    #[arg(long, conflicts_with_all = ["username", "password", "token"])]
    sso_url: Option<String>,
    /// Username for password-based authentication.
    #[arg(long, requires = "password")]
    username: Option<String>,
    /// Password for authentication (will prompt if username provided without password).
    #[arg(long)]
    password: Option<String>,
    /// Local path to download resources to.
    #[arg(long)]
    path: Option<PathBuf>,
    /// Language to force for resource retrieval.
    #[arg(long)]
    lang: Option<String>,
    /// Output format.
    #[arg(long, short, value_enum, default_value_t = OutputFormat::Markdown)]
    output: OutputFormat,
}

/// Extract token from SSO redirect URL.
/// URL format: moodledl://token=BASE64 or moodlemobile://token=BASE64
/// BASE64 decodes to: SIGNATURE:::TOKEN or SIGNATURE:::TOKEN:::PRIVATE_TOKEN
fn extract_token_from_sso_url(sso_url: &str) -> anyhow::Result<Token> {
    // Find the token= part
    let token_part = sso_url
        .split("token=")
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("Invalid SSO URL: missing 'token=' parameter"))?;

    // Remove any trailing parameters or fragments
    let base64_token = token_part.split('&').next().unwrap_or(token_part);
    let base64_token = base64_token.split('#').next().unwrap_or(base64_token);

    // Decode Base64
    let decoded = BASE64_STANDARD
        .decode(base64_token)
        .map_err(|e| anyhow::anyhow!("Failed to decode Base64: {}", e))?;

    let decoded_str = String::from_utf8(decoded)
        .map_err(|e| anyhow::anyhow!("Invalid UTF-8 in decoded token: {}", e))?;

    // Format: SIGNATURE:::TOKEN or SIGNATURE:::TOKEN:::PRIVATE_TOKEN
    // The token is always the second part (index 1)
    let parts: Vec<&str> = decoded_str.split(":::").collect();
    if parts.len() < 2 {
        return Err(anyhow::anyhow!(
            "Invalid token format. Expected 'SIGNATURE:::TOKEN[:::PRIVATE_TOKEN]', got: {}",
            decoded_str
        ));
    }

    let token_hex = parts[1];
    if token_hex.len() != 32 {
        return Err(anyhow::anyhow!(
            "Invalid token length. Expected 32 hex chars, got {} (value: {})",
            token_hex.len(),
            token_hex
        ));
    }

    token_hex
        .parse()
        .map_err(|e| anyhow::anyhow!("Failed to parse token: {}", e))
}

#[derive(Debug, Serialize)]
struct ConfigureResult {
    status: String,
    site: String,
    user: String,
    site_url: String,
    download_path: String,
}

impl Render for ConfigureResult {
    fn to_markdown(&self) -> String {
        format!(
            "Successfully configured Moodle connection.\n\n\
             - **Site**: {}\n\
             - **User**: {}\n\
             - **URL**: {}\n\
             - **Download path**: {}",
            self.site, self.user, self.site_url, self.download_path
        )
    }
}

impl Command {
    pub async fn run(self) -> anyhow::Result<()> {
        let config_task = tokio::spawn(Config::read());

        // Get URL (interactive or from args)
        let url = match self.url {
            Some(url) => url,
            None => {
                let url_str: String = task::spawn_blocking(|| {
                    Input::new()
                        .with_prompt("Moodle server URL")
                        .interact_text()
                })
                .await??;
                url_str.parse()?
            }
        };

        // Get token (via token, sso_url, username/password, or interactive)
        let token = if let Some(token_str) = self.token {
            token_str.parse()?
        } else if let Some(ref sso_url) = self.sso_url {
            match extract_token_from_sso_url(sso_url) {
                Ok(t) => t,
                Err(e) => {
                    print_error(&format!("Failed to extract token from SSO URL: {}", e), self.output);
                    return Err(e);
                }
            }
        } else if let Some(username) = self.username {
            let password = match self.password {
                Some(p) => p,
                None => {
                    task::spawn_blocking(|| Password::new().with_prompt("Password").interact())
                        .await??
                }
            };
            Account::login(&url, &username, &password).await?.token
        } else {
            // Interactive: ask for auth method
            let url_for_sso = url.clone();
            let auth_method = task::spawn_blocking(|| {
                Select::new()
                    .with_prompt("Authentication method")
                    .items(&["Token (direct)", "SSO (redirect URL)", "Username/Password"])
                    .default(0)
                    .interact()
            })
            .await??;

            if auth_method == 0 {
                // Token
                let token_str: String =
                    task::spawn_blocking(|| Password::new().with_prompt("Token (32 hex chars)").interact())
                        .await??;
                token_str.parse()?
            } else if auth_method == 1 {
                // SSO
                eprintln!("\n=== SSO Authentication Instructions ===");
                eprintln!("1. Log into your Moodle account in a web browser");
                eprintln!("2. Open the developer console (press F12) and go to the Network tab");
                eprintln!("3. Visit this URL in the same browser tab:");
                eprintln!("   {}/admin/tool/mobile/launch.php?service=moodle_mobile_app&passport=12345&urlscheme=moodlemobile", url_for_sso.as_str().trim_end_matches('/'));
                eprintln!("4. The page will fail to load - this is expected");
                eprintln!("5. In the Network tab, find the failed request");
                eprintln!("6. Right-click > Copy > Copy link address");
                eprintln!("   (It should look like: moodlemobile://token=...)\n");

                let sso_url: String = task::spawn_blocking(|| {
                    Input::new()
                        .with_prompt("Paste the SSO redirect URL")
                        .interact_text()
                })
                .await??;
                extract_token_from_sso_url(&sso_url)?
            } else {
                // Username/Password
                let username: String =
                    task::spawn_blocking(|| Input::new().with_prompt("Username").interact_text())
                        .await??;
                let password =
                    task::spawn_blocking(|| Password::new().with_prompt("Password").interact())
                        .await??;
                Account::login(&url, &username, &password).await?.token
            }
        };

        // Get download path
        let path = match self.path {
            Some(p) => p,
            None => {
                let default_path = dirs::document_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join("Moodle");
                let path_str: String = task::spawn_blocking(move || {
                    Input::new()
                        .with_prompt("Download path")
                        .default(default_path.display().to_string())
                        .interact_text()
                })
                .await??;
                PathBuf::from(path_str)
            }
        };

        let expanded_path = config::expand_path(&path)?;
        let account_config =
            AccountConfig::new(url, token, expanded_path.clone(), self.lang).await?;

        let result = ConfigureResult {
            status: "ok".to_string(),
            site: account_config.site.clone(),
            user: account_config.user.clone(),
            site_url: account_config.id.site_url.to_string(),
            download_path: expanded_path.display().to_string(),
        };

        let mut config = config_task.await??;
        config
            .accounts
            .insert(account_config.id.to_string(), account_config);
        config.write().await?;

        print_output(&result, self.output);

        Ok(())
    }
}
