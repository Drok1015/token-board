use serde::Serialize;
use serde_json::Value;
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const KIMI_CLIENT_ID: &str = "17e5f671-d194-4dfb-9706-5516cb48c098";

#[derive(Serialize)]
struct QuotaLine {
    provider: &'static str,
    value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    plan: Option<String>,
}

fn home_file(path: &str) -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(path))
}

fn kimi_credential_paths_from_home(home: &Path) -> [PathBuf; 2] {
    [
        home.join(".kimi-code/credentials/kimi-code.json"),
        home.join(".kimi/credentials/kimi-code.json"),
    ]
}

fn kimi_credential_paths() -> Option<[PathBuf; 2]> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    Some(kimi_credential_paths_from_home(&home))
}

fn provider_key(name: &str) -> Option<String> {
    let db = home_file(".cc-switch/cc-switch.db")?;
    let sql = format!("SELECT settings_config FROM providers WHERE app_type='codex' AND name='{name}' LIMIT 1;");
    let output = Command::new("/usr/bin/sqlite3")
        .args([db.to_string_lossy().as_ref(), sql.as_str()])
        .output().ok()?;
    let value: Value = serde_json::from_slice(&output.stdout).ok()?;
    value.pointer("/auth/OPENAI_API_KEY")?.as_str().map(str::to_owned)
}

fn number(value: Option<&Value>) -> Option<i64> {
    value?.as_i64().or_else(|| value?.as_str()?.parse().ok())
}

fn remaining(used: i64, limit: i64) -> String {
    format!("{}%", ((limit - used).clamp(0, limit) * 100 / limit.max(1)))
}

async fn glm_line(client: &reqwest::Client) -> QuotaLine {
    let Some(key) = provider_key("Zhipu GLM") else {
        return QuotaLine { provider: "GLM", value: "未配置".into(), plan: None };
    };
    let response = match client.get("https://open.bigmodel.cn/api/monitor/usage/quota/limit")
        .bearer_auth(key).send().await.and_then(|r| r.error_for_status()) {
        Ok(value) => value,
        Err(_) => return QuotaLine { provider: "GLM", value: "读取失败".into(), plan: None },
    };
    let payload: Value = match response.json().await { Ok(value) => value, Err(_) => return QuotaLine { provider: "GLM", value: "读取失败".into(), plan: None } };
    let mut limits: Vec<&Value> = payload.pointer("/data/limits").and_then(Value::as_array).into_iter().flatten()
        .filter(|item| item["type"].as_str() == Some("TOKENS_LIMIT")).collect();
    limits.sort_by_key(|item| number(item.get("nextResetTime")).unwrap_or(i64::MAX));
    let pct = |item: &Value| format!("{}%", 100 - number(item.get("percentage")).unwrap_or(100));
    let plan = payload.pointer("/data/level").and_then(Value::as_str).map(String::from);
    match (limits.first(), limits.last()) {
        (Some(first), Some(last)) => QuotaLine { provider: "GLM", value: format!("5h {} / 7d {}", pct(first), pct(last)), plan },
        _ => QuotaLine { provider: "GLM", value: "暂无额度".into(), plan },
    }
}

fn now_secs() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

// access_token 有效期仅 15 分钟，过期时用 refresh_token 换新并写回凭据文件
async fn kimi_refresh(client: &reqwest::Client, path: &Path, credential: &mut Value) -> Option<String> {
    let refresh_token = credential["refresh_token"].as_str()?.to_owned();
    let response = client.post("https://auth.kimi.com/api/oauth/token")
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token.as_str()),
            ("client_id", KIMI_CLIENT_ID),
        ])
        .send().await.and_then(|r| r.error_for_status()).ok()?;
    let body: Value = response.json().await.ok()?;
    let token = body["access_token"].as_str()?.to_owned();
    credential["access_token"] = Value::from(token.clone());
    if let Some(new_refresh) = body["refresh_token"].as_str() {
        credential["refresh_token"] = Value::from(new_refresh);
    }
    if let Some(expires_in) = body["expires_in"].as_f64() {
        credential["expires_at"] = Value::from(now_secs() + expires_in);
    }
    if let Ok(data) = serde_json::to_vec(credential) {
        let _ = fs::write(path, data);
    }
    Some(token)
}

async fn kimi_usages(client: &reqwest::Client, token: &str) -> Option<Value> {
    client.get("https://api.kimi.com/coding/v1/usages")
        .bearer_auth(token).send().await.and_then(|r| r.error_for_status())
        .ok()?.json().await.ok()
}

async fn kimi_payload_from_path(client: &reqwest::Client, path: &Path) -> Option<Value> {
    let mut credential: Value = match fs::read(&path).ok().and_then(|data| serde_json::from_slice(&data).ok()) {
        Some(value) => value,
        None => return None,
    };
    let mut token = credential["access_token"].as_str().unwrap_or_default().to_owned();
    let expires_at = credential["expires_at"].as_f64().unwrap_or(0.0);
    if token.is_empty() || expires_at < now_secs() + 60.0 {
        match kimi_refresh(client, path, &mut credential).await {
            Some(new_token) => token = new_token,
            None => return None,
        }
    }
    // 本地未过期但服务端拒绝（如凭据已在别处轮换）时，强制刷新重试一次
    match kimi_usages(client, &token).await {
        Some(value) => Some(value),
        None => match kimi_refresh(client, path, &mut credential).await {
            Some(new_token) => match kimi_usages(client, &new_token).await {
                Some(value) => Some(value),
                None => None,
            },
            None => None,
        },
    }
}

async fn kimi_line(client: &reqwest::Client) -> QuotaLine {
    let Some(paths) = kimi_credential_paths() else {
        return QuotaLine { provider: "KIMI", value: "未登录".into(), plan: None };
    };
    let mut payload = None;
    let mut plan = None;
    for path in paths {
        if let Some(value) = kimi_payload_from_path(client, &path).await {
            plan = value.get("user").and_then(Value::as_object)
                .and_then(|u| u.get("membership").and_then(Value::as_object))
                .and_then(|m| m.get("level").and_then(Value::as_str))
                .map(|level| match level {
                    "LEVEL_FREE" => "Adagio".to_string(),
                    "LEVEL_TRIAL" => "Andante".to_string(),
                    "LEVEL_BASIC" => "Moderato".to_string(),
                    "LEVEL_INTERMEDIATE" => "Allegretto".to_string(),
                    "LEVEL_ADVANCED" => "Allegro".to_string(),
                    other => other.trim_start_matches("LEVEL_").to_string(),
                });
            payload = Some(value);
            break;
        }
    }
    let Some(payload) = payload else {
        return QuotaLine { provider: "KIMI", value: "未登录".into(), plan: None };
    };
    let mut rows: Vec<(String, String)> = vec![];
    if let Some(limits) = payload["limits"].as_array() {
        for item in limits {
            let detail = item.get("detail").unwrap_or(item);
            let window = item.get("window").unwrap_or(&Value::Null);
            let name = item["name"].as_str().or(detail["name"].as_str()).map(str::to_owned).unwrap_or_else(|| {
                let duration = number(window.get("duration")).unwrap_or(0);
                let unit = window["timeUnit"].as_str().unwrap_or("");
                if unit.contains("MINUTE") && duration % 60 == 0 { format!("{}h", duration / 60) } else { "额度".into() }
            });
            if let Some(limit) = number(detail.get("limit")) {
                let used = number(detail.get("used")).or_else(|| number(detail.get("remaining")).map(|v| limit - v)).unwrap_or(0);
                rows.push((name, remaining(used, limit)));
            }
        }
    }
    if let Some(usage) = payload["usage"].as_object() {
        if let Some(limit) = number(usage.get("limit")) {
            let used = number(usage.get("used")).or_else(|| number(usage.get("remaining")).map(|v| limit - v)).unwrap_or(0);
            rows.push(("7d".into(), remaining(used, limit)));
        }
    }
    let five_hour = rows.iter().find(|(name, _)| name.contains("5h") || name.contains("5H")).or_else(|| rows.first());
    let seven_day = rows.iter().find(|(name, _)| name.contains("7d") || name.contains("7D")).or_else(|| rows.get(1));
    match (five_hour, seven_day) {
        (Some((_, h5)), Some((_, d7))) => QuotaLine { provider: "KIMI", value: format!("5h {h5} / 7d {d7}"), plan },
        (Some((name, pct)), None) => QuotaLine { provider: "KIMI", value: format!("{name} {pct}"), plan },
        _ => QuotaLine { provider: "KIMI", value: "暂无额度".into(), plan },
    }
}

async fn deepseek_line(client: &reqwest::Client) -> QuotaLine {
    let Some(key) = provider_key("DeepSeek") else {
        return QuotaLine { provider: "DEEPSEEK", value: "未配置".into(), plan: Some("Token".into()) };
    };
    let response = match client.get("https://api.deepseek.com/user/balance")
        .bearer_auth(key).send().await.and_then(|r| r.error_for_status()) {
        Ok(value) => value,
        Err(_) => return QuotaLine { provider: "DEEPSEEK", value: "读取失败".into(), plan: Some("Token".into()) },
    };
    let payload: Value = match response.json().await { Ok(value) => value, Err(_) => return QuotaLine { provider: "DEEPSEEK", value: "读取失败".into(), plan: Some("Token".into()) } };
    let balance = payload["balance_infos"].as_array().and_then(|items| items.first())
        .and_then(|item| item["total_balance"].as_str()).unwrap_or("—");
    QuotaLine { provider: "DEEPSEEK", value: format!("余额 ¥{balance}"), plan: Some("Token".into()) }
}

fn codex_window(window: &Value) -> Option<(i64, String)> {
    let minutes = number(window.get("windowDurationMins"))?;
    let used = number(window.get("usedPercent")).unwrap_or(100).clamp(0, 100);
    let label = if minutes % 1_440 == 0 {
        format!("{}d", minutes / 1_440)
    } else if minutes % 60 == 0 {
        format!("{}h", minutes / 60)
    } else {
        format!("{minutes}m")
    };
    Some((minutes, format!("{label} {}%", 100 - used)))
}

fn read_codex_limits(cli: &str) -> Option<String> {
    let mut child = match Command::new(cli)
        .args(["app-server", "--stdio"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(value) => value,
        Err(_) => return None,
    };

    let mut stdin = match child.stdin.take() {
        Some(value) => value,
        None => { let _ = child.kill(); return None; }
    };
    let initialize = serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": { "clientInfo": { "name": "Token 看板", "version": "0.2.0" }, "capabilities": { "experimentalApi": true } }
    });
    let initialized = serde_json::json!({
        "jsonrpc": "2.0", "method": "initialized", "params": Value::Null
    });
    let read_limits = serde_json::json!({
        "jsonrpc": "2.0", "id": 2, "method": "account/rateLimits/read", "params": Value::Null
    });
    if writeln!(stdin, "{initialize}").is_err()
        || writeln!(stdin, "{initialized}").is_err()
        || writeln!(stdin, "{read_limits}").is_err()
        || stdin.flush().is_err()
    {
        let _ = child.kill();
        return None;
    }

    let stdout = match child.stdout.take() {
        Some(value) => value,
        None => { let _ = child.kill(); return None; }
    };
    let (tx, rx) = mpsc::channel();
    let _ = thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines().take(64) {
            let Ok(line) = line else { break };
            let Ok(payload) = serde_json::from_str::<Value>(&line) else { continue };
            if payload.get("id").and_then(Value::as_i64) != Some(2) { continue; }
            let Some(limits) = payload.pointer("/result/rateLimits") else { break };
            let mut windows = ["secondary", "primary"].into_iter()
                .filter_map(|name| limits.get(name).and_then(codex_window))
                .collect::<Vec<_>>();
            windows.sort_by_key(|(minutes, _)| *minutes);
            let value = if windows.is_empty() { None } else {
                Some(windows.into_iter().map(|(_, text)| text).collect::<Vec<_>>().join(" / "))
            };
            let _ = tx.send(value);
            break;
        }
    });

    let result = rx.recv_timeout(Duration::from_secs(3)).ok().flatten();
    let _ = child.kill();
    result
}

fn read_codex_plan(cli: &str) -> Option<String> {
    let mut child = match Command::new(cli)
        .args(["app-server", "--stdio"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(value) => value,
        Err(_) => return None,
    };

    let mut stdin = match child.stdin.take() {
        Some(value) => value,
        None => { let _ = child.kill(); return None; }
    };
    let initialize = serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": { "clientInfo": { "name": "Token 看板", "version": "0.2.0" }, "capabilities": { "experimentalApi": true } }
    });
    let initialized = serde_json::json!({
        "jsonrpc": "2.0", "method": "initialized", "params": Value::Null
    });
    let read_account = serde_json::json!({
        "jsonrpc": "2.0", "id": 3, "method": "account/read", "params": { "refreshToken": false }
    });
    if writeln!(stdin, "{initialize}").is_err()
        || writeln!(stdin, "{initialized}").is_err()
        || writeln!(stdin, "{read_account}").is_err()
        || stdin.flush().is_err()
    {
        let _ = child.kill();
        return None;
    }

    let stdout = match child.stdout.take() {
        Some(value) => value,
        None => { let _ = child.kill(); return None; }
    };
    let (tx, rx) = mpsc::channel();
    let _ = thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines().take(64) {
            let Ok(line) = line else { break };
            let Ok(payload) = serde_json::from_str::<Value>(&line) else { continue };
            if payload.get("id").and_then(Value::as_i64) != Some(3) { continue; }
            let plan = payload.pointer("/result/account/planType")
                .and_then(Value::as_str)
                .map(|s| s.to_uppercase_first());
            let _ = tx.send(plan);
            break;
        }
    });

    let result = rx.recv_timeout(Duration::from_secs(3)).ok().flatten();
    let _ = child.kill();
    result
}

fn codex_line() -> QuotaLine {
    let cli = if std::path::Path::new("/Applications/ChatGPT.app/Contents/Resources/codex").is_file() {
        "/Applications/ChatGPT.app/Contents/Resources/codex"
    } else {
        "codex"
    };
    let value = {
        let mut result = None;
        for attempt in 0..2 {
            if let Some(v) = read_codex_limits(cli) {
                result = Some(v);
                break;
            }
            if attempt == 0 { std::thread::sleep(std::time::Duration::from_millis(600)); }
        }
        result
    };
    let plan = read_codex_plan(cli);
    match value {
        Some(v) => QuotaLine { provider: "CODEX", value: v, plan },
        None => QuotaLine { provider: "CODEX", value: "读取失败".into(), plan: None },
    }
}

#[tauri::command]
async fn get_quotas() -> Vec<QuotaLine> {
    let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(10)).build();
    let Ok(client) = client else { return vec![] };
    let codex = tauri::async_runtime::spawn_blocking(codex_line);
    let mut quotas = vec![kimi_line(&client).await, glm_line(&client).await, deepseek_line(&client).await];
    let codex = codex.await.unwrap_or(QuotaLine { provider: "CODEX", value: "读取失败".into(), plan: None });
    quotas.insert(0, codex);
    quotas
}

#[tauri::command]
fn open_app(app: &str) -> Result<(), String> {
    let app_name = match app {
        "huide" => "汇兑", "renren" => "人人视频 for Mac", "parallels" => "Parallels Desktop",
        _ => return Err("不允许打开未配置的应用".into()),
    };
    let status = Command::new("open").args(["-a", app_name]).status()
        .map_err(|error| format!("无法调用 macOS open 命令：{error}"))?;
    if status.success() { Ok(()) } else { Err(format!("未能打开 {app_name}")) }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![open_app, get_quotas])
        .run(tauri::generate_context!())
        .expect("启动 Token 看板失败");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kimi_credentials_prefer_kimi_code_then_legacy_kimi() {
        let paths = kimi_credential_paths_from_home(Path::new("home"));
        assert_eq!(
            paths,
            [
                PathBuf::from("home/.kimi-code/credentials/kimi-code.json"),
                PathBuf::from("home/.kimi/credentials/kimi-code.json"),
            ]
        );
    }
}

// 把 "pro" → "Pro"，"self_serve_business_usage_based" → "Self Serve Business Usage Based"
trait StrExt { fn to_uppercase_first(self) -> String; }
impl StrExt for &str {
    fn to_uppercase_first(self) -> String {
        self.split('_')
            .filter(|part| !part.is_empty())
            .map(|part| {
                let mut chars = part.chars();
                match chars.next() {
                    Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                    None => String::new(),
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
}
