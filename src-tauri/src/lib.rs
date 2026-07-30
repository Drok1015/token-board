use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::{Emitter, Manager};

const KIMI_CLIENT_ID: &str = "17e5f671-d194-4dfb-9706-5516cb48c098";

#[derive(Serialize)]
struct QuotaLine {
    provider: &'static str,
    value: String,
    plan: Option<String>,
}

const ALL_PROVIDERS: [&str; 4] = ["CODEX", "KIMI", "GLM", "DEEPSEEK"];

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
#[serde(default)]
struct BoardSettings {
    auto_hide: bool,
    hide_delay_seconds: u64,
    show_plans: bool,
    visible_providers: Vec<String>,
    glm_api_key: String,
    deepseek_api_key: String,
}

impl Default for BoardSettings {
    fn default() -> Self {
        Self {
            auto_hide: false,
            hide_delay_seconds: 10,
            show_plans: true,
            visible_providers: ALL_PROVIDERS.map(str::to_owned).to_vec(),
            glm_api_key: String::new(),
            deepseek_api_key: String::new(),
        }
    }
}

impl BoardSettings {
    fn normalized(mut self) -> Self {
        self.hide_delay_seconds = self.hide_delay_seconds.clamp(1, 3_600);
        self.visible_providers.retain(|name| ALL_PROVIDERS.contains(&name.as_str()));
        self.glm_api_key = self.glm_api_key.trim().to_owned();
        self.deepseek_api_key = self.deepseek_api_key.trim().to_owned();
        self
    }
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

fn readable_plan_name(value: &str) -> String {
    value
        .trim()
        .trim_start_matches("LEVEL_")
        .split(['_', '-'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn kimi_membership_name(level: &str) -> String {
    match level.trim().to_ascii_uppercase().as_str() {
        "LEVEL_FREE" => "Free".into(),
        "LEVEL_BASIC" => "Adagio".into(),
        "LEVEL_STANDARD" => "Moderato".into(),
        "LEVEL_INTERMEDIATE" => "Allegretto".into(),
        "LEVEL_ADVANCED" => "Allegro".into(),
        "LEVEL_PREMIUM" => "Vivace".into(),
        _ => readable_plan_name(level),
    }
}

// 优先使用设置页填写的 key，留空则回退到 cc-switch
fn provider_key_with_override(name: &str, override_key: &str) -> Option<String> {
    if override_key.is_empty() { provider_key(name) } else { Some(override_key.to_owned()) }
}

async fn glm_line(client: &reqwest::Client, override_key: &str) -> QuotaLine {
    let Some(key) = provider_key_with_override("Zhipu GLM", override_key) else {
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
    let value = match (limits.first(), limits.last()) {
        (Some(first), Some(last)) => format!("5h {} / 7d {}", pct(first), pct(last)),
        _ => "暂无额度".into(),
    };
    let plan = payload.pointer("/data/level").and_then(Value::as_str).map(str::to_owned);
    QuotaLine { provider: "GLM", value, plan }
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
    let mut credential: Value = match fs::read(path).ok().and_then(|data| serde_json::from_slice(&data).ok()) {
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
            Some(new_token) => kimi_usages(client, &new_token).await,
            None => None,
        },
    }
}

async fn kimi_line(client: &reqwest::Client) -> QuotaLine {
    let Some(paths) = kimi_credential_paths() else {
        return QuotaLine { provider: "KIMI", value: "未登录".into(), plan: None };
    };
    let mut payload = None;
    for path in paths {
        if let Some(value) = kimi_payload_from_path(client, &path).await {
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
    let value = match (five_hour, seven_day) {
        (Some((_, h5)), Some((_, d7))) => format!("5h {h5} / 7d {d7}"),
        (Some((name, pct)), None) => format!("{name} {pct}"),
        _ => "暂无额度".into(),
    };
    let plan = payload.pointer("/user/membership/level").and_then(Value::as_str).map(kimi_membership_name);
    QuotaLine { provider: "KIMI", value, plan }
}

async fn deepseek_line(client: &reqwest::Client, override_key: &str) -> QuotaLine {
    let Some(key) = provider_key_with_override("DeepSeek", override_key) else {
        return QuotaLine { provider: "DEEPSEEK", value: "未配置".into(), plan: None };
    };
    let response = match client.get("https://api.deepseek.com/user/balance")
        .bearer_auth(key).send().await.and_then(|r| r.error_for_status()) {
        Ok(value) => value,
        Err(_) => return QuotaLine { provider: "DEEPSEEK", value: "读取失败".into(), plan: None },
    };
    let payload: Value = match response.json().await { Ok(value) => value, Err(_) => return QuotaLine { provider: "DEEPSEEK", value: "读取失败".into(), plan: None } };
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

fn read_codex_limits(cli: &str) -> Option<(String, Option<String>)> {
    let mut child = Command::new(cli)
        .args(["app-server", "--stdio"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    let mut stdin = child.stdin.take()?;
    let initialize = serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": { "clientInfo": { "name": "Token 看板", "version": "0.2.0" }, "capabilities": { "experimentalApi": true } }
    });
    let read_limits = serde_json::json!({
        "jsonrpc": "2.0", "id": 2, "method": "account/rateLimits/read", "params": Value::Null
    });
    if writeln!(stdin, "{initialize}").is_err()
        || writeln!(stdin, "{read_limits}").is_err()
        || stdin.flush().is_err()
    {
        let _ = child.kill();
        return None;
    }

    let stdout = child.stdout.take()?;
    let reader = BufReader::new(stdout);
    let mut value = None;
    for line in reader.lines().take(64) {
        let Ok(line) = line else { break };
        let Ok(payload) = serde_json::from_str::<Value>(&line) else { continue };
        if payload.get("id").and_then(Value::as_i64) != Some(2) { continue; }
        let Some(limits) = payload.pointer("/result/rateLimits") else { break };
        let mut windows = ["secondary", "primary"].into_iter()
            .filter_map(|name| limits.get(name).and_then(codex_window))
            .collect::<Vec<_>>();
        windows.sort_by_key(|(minutes, _)| *minutes);
        if !windows.is_empty() {
            let usage = windows.into_iter().map(|(_, text)| text).collect::<Vec<_>>().join(" / ");
            let plan = limits.get("planType").and_then(Value::as_str).map(readable_plan_name);
            value = Some((usage, plan));
        }
        break;
    }
    let _ = child.kill();
    value
}

fn codex_line() -> QuotaLine {
    let cli = if std::path::Path::new("/Applications/ChatGPT.app/Contents/Resources/codex").is_file() {
        "/Applications/ChatGPT.app/Contents/Resources/codex"
    } else {
        "codex"
    };
    for attempt in 0..2 {
        if let Some((value, plan)) = read_codex_limits(cli) {
            return QuotaLine { provider: "CODEX", value, plan };
        }
        if attempt == 0 { std::thread::sleep(std::time::Duration::from_millis(600)); }
    }
    QuotaLine { provider: "CODEX", value: "读取失败".into(), plan: None }
}

#[tauri::command]
async fn get_quotas(app: tauri::AppHandle) -> Vec<QuotaLine> {
    let settings = read_settings(&app);
    let visible = |name: &str| settings.visible_providers.iter().any(|p| p == name);
    let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(10)).build();
    let Ok(client) = client else { return vec![] };
    let codex = visible("CODEX").then(|| tauri::async_runtime::spawn_blocking(codex_line));
    let kimi = visible("KIMI").then(|| kimi_line(&client));
    let glm = visible("GLM").then(|| glm_line(&client, &settings.glm_api_key));
    let deepseek = visible("DEEPSEEK").then(|| deepseek_line(&client, &settings.deepseek_api_key));
    let mut quotas = vec![];
    if let Some(task) = codex {
        quotas.push(task.await.unwrap_or(QuotaLine { provider: "CODEX", value: "读取失败".into(), plan: None }));
    }
    if let Some(future) = kimi { quotas.push(future.await); }
    if let Some(future) = glm { quotas.push(future.await); }
    if let Some(future) = deepseek { quotas.push(future.await); }
    quotas
}

fn settings_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_config_dir()
        .map(|dir| dir.join("settings.json"))
        .map_err(|error| format!("无法确定设置保存位置：{error}"))
}

fn read_settings(app: &tauri::AppHandle) -> BoardSettings {
    let Ok(path) = settings_path(app) else {
        return BoardSettings::default();
    };
    fs::read(path)
        .ok()
        .and_then(|data| serde_json::from_slice::<BoardSettings>(&data).ok())
        .unwrap_or_default()
        .normalized()
}

#[tauri::command]
fn get_settings(app: tauri::AppHandle) -> BoardSettings {
    read_settings(&app)
}

#[tauri::command]
fn save_settings(app: tauri::AppHandle, settings: BoardSettings) -> Result<(), String> {
    let settings = settings.normalized();
    let path = settings_path(&app)?;
    let parent = path.parent().ok_or_else(|| "设置保存位置无效".to_string())?;
    fs::create_dir_all(parent).map_err(|error| format!("无法创建设置目录：{error}"))?;
    let data = serde_json::to_vec_pretty(&settings).map_err(|error| format!("无法序列化设置：{error}"))?;
    fs::write(path, data).map_err(|error| format!("无法保存设置：{error}"))?;
    app.emit_to("main", "settings-updated", settings)
        .map_err(|error| format!("无法应用设置：{error}"))?;
    if let Some(window) = app.get_webview_window("settings") {
        window.close().map_err(|error| format!("无法关闭设置窗口：{error}"))?;
    }
    Ok(())
}

#[tauri::command]
async fn open_settings(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("settings") {
        window.show().map_err(|error| format!("无法显示设置窗口：{error}"))?;
        window.set_focus().map_err(|error| format!("无法聚焦设置窗口：{error}"))?;
        return Ok(());
    }

    tauri::WebviewWindowBuilder::new(
        &app,
        "settings",
        tauri::WebviewUrl::App("index.html?view=settings".into()),
    )
    .title("设置")
    .inner_size(410.0, 560.0)
    .resizable(false)
    .maximizable(false)
    .minimizable(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .center()
    .build()
    .map_err(|error| format!("无法打开设置窗口：{error}"))?;
    Ok(())
}

#[tauri::command]
fn close_settings(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("settings") {
        window.close().map_err(|error| format!("无法关闭设置窗口：{error}"))?;
    }
    Ok(())
}

#[tauri::command]
fn close_app(app: tauri::AppHandle) {
    app.exit(0);
}

// CODEX 额度回到 100% 时弹系统对话框提醒；osascript 会等用户点击，后台运行不阻塞看板
#[tauri::command]
fn notify_codex_full() {
    let _ = Command::new("osascript")
        .args(["-e", r#"display dialog "codex重置了!!!老铁,抓紧蹬!!" with title "Token 看板" buttons {"知道了"} default button "知道了""#])
        .spawn();
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
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .invoke_handler(tauri::generate_handler![
            open_app,
            get_quotas,
            get_settings,
            save_settings,
            open_settings,
            close_settings,
            close_app,
            notify_codex_full
        ])
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

    #[test]
    fn board_settings_default_to_ten_seconds_and_clamp_invalid_values() {
        let defaults = BoardSettings::default();
        assert!(!defaults.auto_hide);
        assert_eq!(defaults.hide_delay_seconds, 10);
        assert!(defaults.show_plans);
        assert_eq!(
            BoardSettings { auto_hide: true, hide_delay_seconds: 0, ..BoardSettings::default() }
                .normalized()
                .hide_delay_seconds,
            1
        );
        assert_eq!(
            BoardSettings { auto_hide: true, hide_delay_seconds: 9_999, ..BoardSettings::default() }
                .normalized()
                .hide_delay_seconds,
            3_600
        );
    }

    #[test]
    fn legacy_settings_keep_existing_values_and_enable_plan_labels() {
        let settings: BoardSettings = serde_json::from_str(
            r#"{"autoHide":true,"hideDelaySeconds":23}"#,
        )
        .expect("legacy settings should remain readable");
        assert!(settings.auto_hide);
        assert_eq!(settings.hide_delay_seconds, 23);
        assert!(settings.show_plans);
        assert_eq!(settings.visible_providers, ALL_PROVIDERS.map(str::to_owned).to_vec());
        assert!(settings.glm_api_key.is_empty());
        assert!(settings.deepseek_api_key.is_empty());
    }

    #[test]
    fn normalized_settings_drop_unknown_providers_and_trim_api_keys() {
        let settings = BoardSettings {
            visible_providers: vec!["GLM".into(), "UNKNOWN".into(), "KIMI".into()],
            glm_api_key: "  sk-glm  ".into(),
            deepseek_api_key: " sk-deepseek ".into(),
            ..BoardSettings::default()
        }
        .normalized();
        assert_eq!(settings.visible_providers, ["GLM", "KIMI"]);
        assert_eq!(settings.glm_api_key, "sk-glm");
        assert_eq!(settings.deepseek_api_key, "sk-deepseek");
    }

    #[test]
    fn provider_plan_names_are_readable() {
        assert_eq!(readable_plan_name("plus"), "Plus");
        assert_eq!(readable_plan_name("enterprise_team"), "Enterprise Team");
        assert_eq!(kimi_membership_name("LEVEL_INTERMEDIATE"), "Allegretto");
        assert_eq!(kimi_membership_name("LEVEL_PREMIUM"), "Vivace");
    }
}
