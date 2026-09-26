use serde::Serialize;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const TOKEN_URLS: [&str; 2] = [
    "https://platform.claude.com/v1/oauth/token",
    "https://console.anthropic.com/v1/oauth/token",
];
const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
/// The usage API rate-limits requests without a claude-code User-Agent.
const USER_AGENT: &str = "claude-code/2.1.118";
/// The token endpoint answers 429 to claude-code/* User-Agents but accepts the
/// axios client that Claude Code itself uses for refreshes.
const REFRESH_USER_AGENT: &str = "axios/1.8.4";
#[cfg(target_os = "macos")]
const KEYCHAIN_SERVICE: &str = "Claude Code-credentials";
/// Refresh this long before the token actually expires.
const EXPIRY_MARGIN_MS: i64 = 5 * 60 * 1000;

#[derive(Serialize)]
struct UsageResult {
    /// Raw /api/oauth/usage response (five_hour, seven_day, ...).
    usage: Value,
    /// Where the credentials were read from, for display.
    source: String,
}

/// Where credentials live: a JSON file, or (macOS only) the login Keychain.
enum Store {
    File(PathBuf),
    #[cfg(target_os = "macos")]
    Keychain { account: String },
}

impl Store {
    fn describe(&self) -> String {
        match self {
            Store::File(p) => p.display().to_string(),
            #[cfg(target_os = "macos")]
            Store::Keychain { .. } => "macOS Keychain".into(),
        }
    }

    fn read(&self) -> Result<Value, String> {
        let text = match self {
            Store::File(p) => fs::read_to_string(p)
                .map_err(|e| format!("Could not read {}: {e}", p.display()))?,
            #[cfg(target_os = "macos")]
            Store::Keychain { .. } => keychain::read()?.1,
        };
        serde_json::from_str(&text).map_err(|e| format!("Credentials are not valid JSON: {e}"))
    }

    fn write(&self, creds: &Value) -> Result<(), String> {
        let text = serde_json::to_string(creds).map_err(|e| e.to_string())?;
        match self {
            Store::File(p) => {
                // Write to a temp file then rename, so Claude Code never sees a half-written file.
                let tmp = p.with_extension("json.tmp");
                fs::write(&tmp, &text).map_err(|e| format!("Could not write credentials: {e}"))?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600));
                }
                fs::rename(&tmp, p).map_err(|e| format!("Could not replace credentials: {e}"))
            }
            #[cfg(target_os = "macos")]
            Store::Keychain { account } => keychain::write(account, &text),
        }
    }
}

#[cfg(target_os = "macos")]
mod keychain {
    use super::KEYCHAIN_SERVICE;
    use std::process::Command;

    /// Returns (account, secret) for the Claude Code Keychain item.
    pub fn read() -> Result<(String, String), String> {
        let attrs = Command::new("security")
            .args(["find-generic-password", "-s", KEYCHAIN_SERVICE])
            .output()
            .map_err(|e| e.to_string())?;
        if !attrs.status.success() {
            return Err("No Claude Code credentials found in the macOS Keychain".into());
        }
        let attrs = String::from_utf8_lossy(&attrs.stdout);
        let account = attrs
            .lines()
            .find_map(|l| l.trim().strip_prefix("\"acct\"<blob>=\""))
            .and_then(|rest| rest.strip_suffix('"'))
            .unwrap_or_default()
            .to_string();
        let secret = Command::new("security")
            .args(["find-generic-password", "-s", KEYCHAIN_SERVICE, "-w"])
            .output()
            .map_err(|e| e.to_string())?;
        if !secret.status.success() {
            return Err("Access to the Claude Code Keychain item was denied".into());
        }
        Ok((account, String::from_utf8_lossy(&secret.stdout).trim().to_string()))
    }

    pub fn write(account: &str, secret: &str) -> Result<(), String> {
        security_framework::passwords::set_generic_password(
            KEYCHAIN_SERVICE,
            account,
            secret.as_bytes(),
        )
        .map_err(|e| format!("Could not update Keychain: {e}"))
    }
}

/// Claude Code's config dir: $CLAUDE_CONFIG_DIR, else ~/.claude.
fn config_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("CLAUDE_CONFIG_DIR").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    dirs::home_dir().map(|h| h.join(".claude"))
}

fn default_credentials_path() -> Option<PathBuf> {
    config_dir().map(|d| d.join(".credentials.json"))
}

/// Returns the standard credentials file path if it exists, else None.
#[tauri::command]
fn detect_token_path() -> Option<String> {
    default_credentials_path()
        .filter(|p| p.is_file())
        .map(|p| p.display().to_string())
}

/// Folder to open the file picker in.
#[tauri::command]
fn default_token_dir() -> Option<String> {
    config_dir()
        .filter(|d| d.is_dir())
        .or_else(dirs::home_dir)
        .map(|d| d.display().to_string())
}

fn resolve_store(path: Option<String>) -> Result<Store, String> {
    let path = path.map(|p| p.trim().to_string()).filter(|p| !p.is_empty());
    if let Some(p) = path {
        let p = PathBuf::from(p);
        if !p.is_file() {
            return Err(format!("Token file not found: {}", p.display()));
        }
        return Ok(Store::File(p));
    }
    #[cfg(target_os = "macos")]
    {
        let (account, _) = keychain::read()?;
        return Ok(Store::Keychain { account });
    }
    #[allow(unreachable_code)]
    Err("No token file found. Log in with `claude`, or choose the file with \"Find token path\".".into())
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn oauth(creds: &Value) -> Result<&Value, String> {
    creds
        .get("claudeAiOauth")
        .filter(|v| v.is_object())
        .ok_or_else(|| "Credentials have no claudeAiOauth section (are you logged in with a Claude subscription?)".into())
}

fn access_token(creds: &Value) -> Result<String, String> {
    oauth(creds)?
        .get("accessToken")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| "Credentials have no access token".into())
}

fn is_expired(creds: &Value) -> bool {
    oauth(creds)
        .ok()
        .and_then(|o| o.get("expiresAt"))
        .and_then(Value::as_i64)
        .map(|exp| exp - EXPIRY_MARGIN_MS <= now_ms())
        .unwrap_or(false)
}

/// Directory-based lock shared with the Claude Code CLI, so two processes never
/// spend the same single-use refresh token.
struct RefreshLock(Option<PathBuf>);

impl RefreshLock {
    fn acquire() -> Self {
        let Some(path) = config_dir().map(|d| d.join(".oauth_refresh.lock")) else {
            return RefreshLock(None);
        };
        for _ in 0..50 {
            if fs::create_dir(&path).is_ok() {
                return RefreshLock(Some(path));
            }
            // Treat locks older than 60s as abandoned.
            let stale = fs::metadata(&path)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|age| age > Duration::from_secs(60));
            if stale {
                let _ = fs::remove_dir(&path);
                continue;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        RefreshLock(None)
    }
}

impl Drop for RefreshLock {
    fn drop(&mut self) {
        if let Some(p) = &self.0 {
            let _ = fs::remove_dir(p);
        }
    }
}

/// Exchanges the refresh token for a new access token and writes the rotated
/// tokens back to the store so Claude Code keeps working.
async fn refresh(client: &reqwest::Client, store: &Store, stale_token: &str) -> Result<Value, String> {
    let _lock = tokio_block(RefreshLock::acquire).await;

    // Someone else (usually Claude Code) may have refreshed while we waited.
    let mut creds = store.read()?;
    if access_token(&creds)? != stale_token && !is_expired(&creds) {
        return Ok(creds);
    }

    let o = oauth(&creds)?;
    let refresh_token = o
        .get("refreshToken")
        .and_then(Value::as_str)
        .ok_or("Token expired and no refresh token is stored. Run `claude` to log in again.")?;
    let mut body = serde_json::json!({
        "grant_type": "refresh_token",
        "refresh_token": refresh_token,
        "client_id": CLIENT_ID,
    });
    if let Some(scopes) = o.get("scopes").and_then(Value::as_array) {
        let scopes: Vec<&str> = scopes.iter().filter_map(Value::as_str).collect();
        if !scopes.is_empty() {
            body["scope"] = scopes.join(" ").into();
        }
    }

    let mut last_err = String::from("Token refresh failed");
    for url in TOKEN_URLS {
        let resp = match client
            .post(url)
            .header(reqwest::header::USER_AGENT, REFRESH_USER_AGENT)
            .json(&body)
            .send()
            .await {
            Ok(r) => r,
            Err(e) => {
                last_err = format!("Token refresh failed: {e}");
                continue;
            }
        };
        let status = resp.status();
        if matches!(status.as_u16(), 404 | 405) || status.is_redirection() {
            last_err = format!("Token refresh failed: HTTP {status}");
            continue;
        }
        if status.as_u16() == 429 {
            return Err("Token refresh is rate limited; will retry shortly.".into());
        }
        if !status.is_success() {
            return Err(format!(
                "Token refresh was rejected (HTTP {status}). Run `claude` to log in again."
            ));
        }
        let tok: Value = resp.json().await.map_err(|e| e.to_string())?;
        let new_access = tok
            .get("access_token")
            .and_then(Value::as_str)
            .ok_or("Token refresh response had no access_token")?;

        let o = creds["claudeAiOauth"].as_object_mut().unwrap();
        o.insert("accessToken".into(), new_access.into());
        if let Some(r) = tok.get("refresh_token").and_then(Value::as_str) {
            o.insert("refreshToken".into(), r.into());
        }
        if let Some(secs) = tok.get("expires_in").and_then(Value::as_i64) {
            o.insert("expiresAt".into(), (now_ms() + secs * 1000).into());
        }
        if let Some(scope) = tok.get("scope").and_then(Value::as_str).filter(|s| !s.trim().is_empty()) {
            o.insert("scopes".into(), scope.split_whitespace().collect::<Vec<_>>().into());
        }
        store.write(&creds)?;
        return Ok(creds);
    }
    Err(last_err)
}

async fn tokio_block<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .expect("blocking task panicked")
}

async fn fetch_usage(client: &reqwest::Client, token: &str) -> Result<(reqwest::StatusCode, String), String> {
    let resp = client
        .get(USAGE_URL)
        .bearer_auth(token)
        .header("anthropic-beta", "oauth-2025-04-20")
        .send()
        .await
        .map_err(|e| format!("Network error: {e}"))?;
    let status = resp.status();
    let text = resp.text().await.map_err(|e| e.to_string())?;
    Ok((status, text))
}

#[tauri::command]
async fn get_usage(path: Option<String>) -> Result<UsageResult, String> {
    let store = resolve_store(path)?;
    let client = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?;

    let mut creds = store.read()?;
    let mut token = access_token(&creds)?;
    if is_expired(&creds) {
        creds = refresh(&client, &store, &token).await?;
        token = access_token(&creds)?;
    }

    let (mut status, mut text) = fetch_usage(&client, &token).await?;
    if status.as_u16() == 401 {
        creds = refresh(&client, &store, &token).await?;
        token = access_token(&creds)?;
        (status, text) = fetch_usage(&client, &token).await?;
    }
    if status.as_u16() == 429 {
        return Err("Rate limited by the usage API; will retry shortly.".into());
    }
    if !status.is_success() {
        let msg = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|v| v.pointer("/error/message").and_then(Value::as_str).map(str::to_string))
            .unwrap_or(text);
        return Err(format!("Usage API error (HTTP {status}): {msg}"));
    }
    let usage: Value = serde_json::from_str(&text).map_err(|e| format!("Bad usage response: {e}"))?;
    Ok(UsageResult { usage, source: store.describe() })
}

/// True when the path points at an existing file (used to validate the textbox).
#[tauri::command]
fn path_exists(path: String) -> bool {
    Path::new(path.trim()).is_file()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            detect_token_path,
            default_token_dir,
            get_usage,
            path_exists
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
