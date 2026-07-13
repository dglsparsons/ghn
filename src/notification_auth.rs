use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

#[cfg(not(target_os = "macos"))]
use std::io::{self, Write};

use anyhow::{anyhow, Context, Result};
use reqwest::Client;
use serde::Deserialize;

use crate::util::open_in_browser;

const MOBILE_CLIENT_ID: &str = "3f8b8834a91f0caad392";
// Native-app credentials are shipped to every GitHub Mobile install; this value
// selects Mobile's private schema but cannot provide confidentiality by itself.
const MOBILE_CLIENT_SECRET: &str = "00e76fc8358899d7795a46cd04ace865fcdc0165";
const CALLBACK_URL: &str = "github://com.github.android/oauth";
const KEYCHAIN_SERVICE: &str = "ghn-github-notifications";
const KEYCHAIN_ACCOUNT: &str = "github.com";

#[derive(Debug, Deserialize)]
struct AccessTokenResponse {
    access_token: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

pub async fn notification_token(client: &Client, reauthorize: bool) -> Result<String> {
    if let Ok(token) = std::env::var("GHN_NOTIFICATIONS_TOKEN") {
        let token = token.trim();
        if !token.is_empty() {
            return Ok(token.to_string());
        }
    }

    if !reauthorize {
        if let Some(token) = load_stored_token()? {
            return Ok(token);
        }
    }

    let token = authorize(client).await?;
    store_token(&token)?;
    Ok(token)
}

async fn authorize(client: &Client) -> Result<String> {
    let state = random_hex(32)?;
    let authorize_url = format!(
        "https://github.com/login/oauth/authorize?client_id={MOBILE_CLIENT_ID}&redirect_uri=github%3A%2F%2Fcom.github.android%2Foauth&scope=notifications&state={state}"
    );

    eprintln!("ghn needs one-time access to GitHub's exact notification Inbox.");

    #[cfg(target_os = "macos")]
    let callback = capture_macos_callback(&authorize_url, &state).await?;

    #[cfg(not(target_os = "macos"))]
    let callback = capture_manual_callback(&authorize_url)?;

    let (code, returned_state) = parse_callback(&callback)?;
    if returned_state != state {
        return Err(anyhow!(
            "GitHub OAuth state did not match; authorization cancelled"
        ));
    }

    let response = client
        .post("https://github.com/login/oauth/access_token")
        .header("Accept", "application/json")
        .header("User-Agent", "ghn")
        .form(&[
            ("client_id", MOBILE_CLIENT_ID),
            ("client_secret", MOBILE_CLIENT_SECRET),
            ("code", code.as_str()),
            ("redirect_uri", CALLBACK_URL),
            ("state", state.as_str()),
        ])
        .send()
        .await
        .context("failed to exchange GitHub notification authorization")?;
    if !response.status().is_success() {
        return Err(anyhow!(
            "GitHub notification authorization failed: {}",
            response.status()
        ));
    }
    let payload: AccessTokenResponse = response.json().await?;
    if let Some(token) = payload.access_token.filter(|token| !token.is_empty()) {
        return Ok(token);
    }

    Err(anyhow!(
        "GitHub notification authorization failed: {}",
        payload
            .error_description
            .or(payload.error)
            .unwrap_or_else(|| "no access token returned".to_string())
    ))
}

#[cfg(target_os = "macos")]
async fn capture_macos_callback(authorize_url: &str, state: &str) -> Result<String> {
    let home = std::env::var_os("HOME").ok_or_else(|| anyhow!("HOME is not set"))?;
    let temp_dir = PathBuf::from(home)
        .join("Library/Caches/ghn")
        .join(format!("oauth-{state}"));
    fs::create_dir_all(&temp_dir).context("failed to create OAuth callback directory")?;
    let helper = match MacCallbackHelper::install(&temp_dir) {
        Ok(helper) => helper,
        Err(error) => {
            let _ = fs::remove_dir_all(&temp_dir);
            return Err(error);
        }
    };

    eprintln!("Opening GitHub authorization in your browser...");
    open_in_browser(authorize_url)?;

    for _ in 0..600 {
        if let Ok(value) = fs::read_to_string(&helper.callback_path) {
            let value = value.trim();
            if !value.is_empty() {
                return Ok(value.to_string());
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    Err(anyhow!("timed out waiting for GitHub authorization"))
}

#[cfg(not(target_os = "macos"))]
fn capture_manual_callback(authorize_url: &str) -> Result<String> {
    open_in_browser(authorize_url)?;
    eprint!("Paste the github:// callback URL here: ");
    io::stderr().flush().ok();
    let mut callback = String::new();
    io::stdin().read_line(&mut callback)?;
    Ok(callback.trim().to_string())
}

#[cfg(target_os = "macos")]
struct MacCallbackHelper {
    root: PathBuf,
    app_path: PathBuf,
    callback_path: PathBuf,
}

#[cfg(target_os = "macos")]
impl MacCallbackHelper {
    fn install(root: &Path) -> Result<Self> {
        let app_path = root.join("ghn OAuth Callback.app");
        let source_path = root.join("callback.applescript");
        let callback_path = root.join("callback.txt");
        let script = format!(
            "on open location callbackUrl\n  do shell script \"/usr/bin/printf '%s' \" & quoted form of callbackUrl & \" > \" & quoted form of \"{}\"\nend open location\n",
            callback_path.display()
        );
        fs::write(&source_path, script).context("failed to write OAuth callback helper")?;

        run_command(
            Command::new("osacompile")
                .args(["-o"])
                .arg(&app_path)
                .arg(&source_path),
            "failed to compile OAuth callback helper",
        )?;

        let plist = app_path.join("Contents/Info.plist");
        let plist_buddy = "/usr/libexec/PlistBuddy";
        run_command(
            Command::new(plist_buddy)
                .args([
                    "-c",
                    "Add :CFBundleIdentifier string com.github.ghn.oauth-callback",
                ])
                .arg(&plist),
            "failed to identify OAuth callback helper",
        )?;
        for command in [
            "Add :CFBundleURLTypes array",
            "Add :CFBundleURLTypes:0 dict",
            "Add :CFBundleURLTypes:0:CFBundleURLName string com.github.ghn.oauth-callback",
            "Add :CFBundleURLTypes:0:CFBundleURLSchemes array",
            "Add :CFBundleURLTypes:0:CFBundleURLSchemes:0 string github",
        ] {
            run_command(
                Command::new(plist_buddy).args(["-c", command]).arg(&plist),
                "failed to configure OAuth callback helper",
            )?;
        }

        run_command(
            Command::new(lsregister())
                .args(["-f", "-R", "-trusted"])
                .arg(&app_path),
            "failed to register OAuth callback helper",
        )?;

        Ok(Self {
            root: root.to_path_buf(),
            app_path,
            callback_path,
        })
    }
}

#[cfg(target_os = "macos")]
impl Drop for MacCallbackHelper {
    fn drop(&mut self) {
        let _ = Command::new(lsregister())
            .arg("-u")
            .arg(&self.app_path)
            .output();
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[cfg(target_os = "macos")]
fn lsregister() -> &'static str {
    "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister"
}

#[cfg(target_os = "macos")]
fn run_command(command: &mut Command, context: &str) -> Result<()> {
    let output = command.output().with_context(|| context.to_string())?;
    if output.status.success() {
        return Ok(());
    }
    Err(anyhow!(
        "{}: {}",
        context,
        String::from_utf8_lossy(&output.stderr).trim()
    ))
}

fn parse_callback(callback: &str) -> Result<(String, String)> {
    if !callback.starts_with(CALLBACK_URL) {
        return Err(anyhow!("unexpected GitHub OAuth callback"));
    }
    let query = callback
        .split_once('?')
        .map(|(_, query)| query)
        .ok_or_else(|| anyhow!("GitHub OAuth callback contained no authorization code"))?;
    let mut code = None;
    let mut state = None;
    for pair in query.split('&') {
        let Some((key, value)) = pair.split_once('=') else {
            continue;
        };
        match key {
            "code" => code = Some(value.to_string()),
            "state" => state = Some(value.to_string()),
            _ => {}
        }
    }
    Ok((
        code.ok_or_else(|| anyhow!("GitHub OAuth callback contained no authorization code"))?,
        state.ok_or_else(|| anyhow!("GitHub OAuth callback contained no state"))?,
    ))
}

fn random_hex(bytes: usize) -> Result<String> {
    let mut input =
        fs::File::open("/dev/urandom").context("failed to open system random source")?;
    let mut random = vec![0u8; bytes];
    input
        .read_exact(&mut random)
        .context("failed to read system random source")?;
    Ok(random.iter().map(|byte| format!("{byte:02x}")).collect())
}

#[cfg(target_os = "macos")]
fn load_stored_token() -> Result<Option<String>> {
    let output = Command::new("security")
        .args([
            "find-generic-password",
            "-a",
            KEYCHAIN_ACCOUNT,
            "-s",
            KEYCHAIN_SERVICE,
            "-w",
        ])
        .output()
        .context("failed to read notification token from Keychain")?;
    if !output.status.success() {
        return Ok(None);
    }
    let token = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok((!token.is_empty()).then_some(token))
}

#[cfg(target_os = "macos")]
fn store_token(token: &str) -> Result<()> {
    let output = Command::new("security")
        .args([
            "add-generic-password",
            "-U",
            "-a",
            KEYCHAIN_ACCOUNT,
            "-s",
            KEYCHAIN_SERVICE,
            "-w",
            token,
        ])
        .output()
        .context("failed to store notification token in Keychain")?;
    if output.status.success() {
        return Ok(());
    }
    Err(anyhow!(
        "failed to store notification token in Keychain: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    ))
}

#[cfg(not(target_os = "macos"))]
fn load_stored_token() -> Result<Option<String>> {
    Ok(None)
}

#[cfg(not(target_os = "macos"))]
fn store_token(_token: &str) -> Result<()> {
    Err(anyhow!(
        "automatic credential storage is not available on this platform; set GHN_NOTIFICATIONS_TOKEN"
    ))
}

#[cfg(test)]
mod tests {
    use super::parse_callback;

    #[test]
    fn parses_oauth_callback() {
        assert_eq!(
            parse_callback("github://com.github.android/oauth?code=abc123&state=def456").unwrap(),
            ("abc123".to_string(), "def456".to_string())
        );
    }

    #[test]
    fn rejects_other_callback_scheme() {
        assert!(parse_callback("https://example.com/?code=x&state=y").is_err());
    }
}
