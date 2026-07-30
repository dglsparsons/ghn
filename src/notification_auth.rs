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
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use reqwest::Client;
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::types::GraphQlResponse;
use crate::util::open_in_browser;

const MOBILE_CLIENT_ID: &str = "3f8b8834a91f0caad392";
// Native-app credentials are shipped to every GitHub Mobile install; this value
// selects Mobile's private schema but cannot provide confidentiality by itself.
const MOBILE_CLIENT_SECRET: &str = "00e76fc8358899d7795a46cd04ace865fcdc0165";
const CALLBACK_URL: &str = "github://com.github.android/oauth";
const KEYCHAIN_SERVICE: &str = "ghn-github-notifications";
const KEYCHAIN_ACCOUNT: &str = "github.com";
const GITHUB_GRAPHQL: &str = "https://api.github.com/graphql";
const GITHUB_USER_API: &str = "https://api.github.com/user";
const REQUIRED_SCOPES: [&str; 2] = ["repo", "notifications"];

#[derive(Debug, Deserialize)]
struct AccessTokenResponse {
    access_token: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

pub async fn github_token(client: &Client, reauthorize: bool) -> Result<String> {
    if let Some((name, token)) = token_from_environment() {
        if token_has_required_access(client, &token).await? {
            return Ok(token);
        }
        return Err(anyhow!(
            "{name} must be Mobile-issued and grant the 'repo' and 'notifications' scopes"
        ));
    }

    if !reauthorize {
        if let Some(token) = load_stored_token()? {
            if token_has_required_access(client, &token).await? {
                return Ok(token);
            }
            eprintln!("ghn needs to upgrade its stored GitHub authorization.");
        }
    }

    let token = authorize(client).await?;
    if !token_has_required_access(client, &token).await? {
        return Err(anyhow!(
            "GitHub authorization did not grant the required access"
        ));
    }
    store_token(&token)?;
    Ok(token)
}

fn token_from_environment() -> Option<(String, String)> {
    ["GHN_TOKEN", "GHN_NOTIFICATIONS_TOKEN"]
        .into_iter()
        .find_map(|name| {
            let token = std::env::var(name).ok()?;
            let token = token.trim();
            (!token.is_empty()).then(|| (name.to_string(), token.to_string()))
        })
}

async fn token_has_required_access(client: &Client, token: &str) -> Result<bool> {
    let response = client
        .get(GITHUB_USER_API)
        .bearer_auth(token)
        .header("User-Agent", "ghn")
        .send()
        .await
        .context("failed to validate GitHub authorization")?;
    if response.status() == 401 {
        return Ok(false);
    }
    if !response.status().is_success() {
        return Err(anyhow!(
            "failed to validate GitHub authorization: {}",
            response.status()
        ));
    }

    let scopes = response
        .headers()
        .get("x-oauth-scopes")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    if !has_required_scopes(scopes) {
        return Ok(false);
    }

    token_has_mobile_inbox_access(client, token).await
}

async fn token_has_mobile_inbox_access(client: &Client, token: &str) -> Result<bool> {
    let response = client
        .post(GITHUB_GRAPHQL)
        .bearer_auth(token)
        .header("User-Agent", "GitHub-Android/1.267.0")
        .json(&json!({
            "query": "query GHNAuthorizationCheck { viewer { notificationThreads(first: 1, query: \"is:unread\") { nodes { id } } } }"
        }))
        .send()
        .await
        .context("failed to validate GitHub Mobile authorization")?;
    if response.status() == 401 {
        return Ok(false);
    }
    if !response.status().is_success() {
        return Err(anyhow!(
            "failed to validate GitHub Mobile authorization: {}",
            response.status()
        ));
    }

    let payload: GraphQlResponse<serde_json::Value> = response
        .json()
        .await
        .context("failed to decode GitHub Mobile authorization check")?;
    let has_errors = payload
        .errors
        .as_ref()
        .map(|errors| !errors.is_empty())
        .unwrap_or(false);
    let has_inbox = payload
        .data
        .as_ref()
        .and_then(|data| data.pointer("/viewer/notificationThreads"))
        .is_some_and(|inbox| !inbox.is_null());
    Ok(has_inbox && !has_errors)
}

fn has_required_scopes(scopes: &str) -> bool {
    let scopes: Vec<_> = scopes.split(',').map(str::trim).collect();
    REQUIRED_SCOPES
        .iter()
        .all(|required| scopes.contains(required))
}

async fn authorize(client: &Client) -> Result<String> {
    let state = random_hex(32)?;
    let code_verifier = random_hex(32)?;
    let code_challenge = pkce_challenge(&code_verifier);
    let authorize_url = format!(
        "https://github.com/login/oauth/authorize?client_id={MOBILE_CLIENT_ID}&redirect_uri=github%3A%2F%2Fcom.github.android%2Foauth&scope=repo%20notifications&state={state}&code_challenge={code_challenge}&code_challenge_method=S256"
    );

    eprintln!("ghn needs one-time access to your GitHub repositories and notification Inbox.");

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
            ("code_verifier", code_verifier.as_str()),
        ])
        .send()
        .await
        .context("failed to exchange GitHub authorization")?;
    if !response.status().is_success() {
        return Err(anyhow!(
            "GitHub authorization failed: {}",
            response.status()
        ));
    }
    let payload: AccessTokenResponse = response.json().await?;
    if let Some(token) = payload.access_token.filter(|token| !token.is_empty()) {
        return Ok(token);
    }

    Err(anyhow!(
        "GitHub authorization failed: {}",
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

fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
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
        .context("failed to read GitHub token from Keychain")?;
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
        .context("failed to store GitHub token in Keychain")?;
    if output.status.success() {
        return Ok(());
    }
    Err(anyhow!(
        "failed to store GitHub token in Keychain: {}",
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
        "automatic credential storage is not available on this platform; set GHN_TOKEN"
    ))
}

#[cfg(test)]
mod tests {
    use super::{has_required_scopes, parse_callback, pkce_challenge};

    #[test]
    fn generates_rfc7636_pkce_challenge() {
        assert_eq!(
            pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn accepts_required_oauth_scopes() {
        assert!(has_required_scopes("repo, notifications"));
        assert!(has_required_scopes("notifications, repo, user"));
    }

    #[test]
    fn rejects_incomplete_oauth_scopes() {
        assert!(!has_required_scopes("notifications"));
        assert!(!has_required_scopes("public_repo, notifications"));
        assert!(!has_required_scopes(""));
    }

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
