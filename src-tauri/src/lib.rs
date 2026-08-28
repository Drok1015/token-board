use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashSet,
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{mpsc, Mutex, OnceLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::{
    menu::{CheckMenuItemBuilder, Menu, MenuBuilder, MenuItemBuilder},
    tray::TrayIconBuilder,
    Emitter, Manager,
};

const KIMI_CLIENT_ID: &str = "17e5f671-d194-4dfb-9706-5516cb48c098";
const CODEX_CHILD_TIMEOUT: Duration = Duration::from_secs(12);

struct QuotaHttpClient(reqwest::Client);

impl Default for QuotaHttpClient {
    fn default() -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .pool_idle_timeout(Duration::from_secs(10 * 60))
            .pool_max_idle_per_host(1)
            .build()
            .expect("构建额度 HTTP 客户端失败");
        Self(client)
    }
}

fn active_child_pids() -> &'static Mutex<HashSet<u32>> {
    static PIDS: OnceLock<Mutex<HashSet<u32>>> = OnceLock::new();
    PIDS.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Owns a child process, places it in an isolated process group and always reaps it.
/// The PID registry lets the app terminate in-flight children before its event loop exits.
struct ManagedChild {
    child: Child,
    pid: u32,
    finished: bool,
}

impl ManagedChild {
    fn spawn(command: &mut Command) -> std::io::Result<Self> {
        command.process_group(0);
        let child = command.spawn()?;
        let pid = child.id();
        active_child_pids().lock().unwrap().insert(pid);
        Ok(Self {
            child,
            pid,
            finished: false,
        })
    }

    fn terminate_and_wait(&mut self) {
        if self.finished {
            return;
        }
        // Kill the entire process group so a helper inherited from the child cannot outlive the app.
        unsafe {
            libc::kill(-(self.pid as i32), libc::SIGKILL);
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.finished = true;
        active_child_pids().lock().unwrap().remove(&self.pid);
    }

    fn wait(&mut self) {
        if self.finished {
            return;
        }
        let _ = self.child.wait();
        self.finished = true;
        active_child_pids().lock().unwrap().remove(&self.pid);
    }
}

impl Drop for ManagedChild {
    fn drop(&mut self) {
        self.terminate_and_wait();
    }
}

fn terminate_active_children() {
    let pids = active_child_pids()
        .lock()
        .unwrap()
        .iter()
        .copied()
        .collect::<Vec<_>>();
    for pid in pids {
        unsafe {
            libc::kill(-(pid as i32), libc::SIGTERM);
            libc::kill(pid as i32, libc::SIGTERM);
        }
    }
}

// 额度窗口的重置时间，用于看板沙漏图标的悬浮提示（目前只有 CODEX 提供）
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct QuotaReset {
    label: String,
    resets_at_ms: i64,
}

#[derive(Serialize)]
struct QuotaLine {
    provider: &'static str,
    value: String,
    plan: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    resets: Vec<QuotaReset>,
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
    auto_update: bool,
    tray_provider: String,
    codex_alert: bool,
    show_board: bool,
    show_tray: bool,
    show_resets: bool,
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
            auto_update: true,
            tray_provider: "CODEX".into(),
            codex_alert: true,
            show_board: true,
            show_tray: true,
            show_resets: true,
        }
    }
}

impl BoardSettings {
    fn normalized(mut self) -> Self {
        self.hide_delay_seconds = self.hide_delay_seconds.clamp(1, 3_600);
        self.visible_providers.retain(|name| ALL_PROVIDERS.contains(&name.as_str()));
        self.glm_api_key = self.glm_api_key.trim().to_owned();
        self.deepseek_api_key = self.deepseek_api_key.trim().to_owned();
        if !ALL_PROVIDERS.contains(&self.tray_provider.as_str()) {
            self.tray_provider = "CODEX".into();
        }
        // 面板和任务栏至少保留一个显示位置
        if !self.show_board && !self.show_tray {
            self.show_board = true;
        }
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
        return QuotaLine { provider: "GLM", value: "未配置".into(), plan: None, resets: Vec::new() };
    };
    let response = match client.get("https://open.bigmodel.cn/api/monitor/usage/quota/limit")
        .bearer_auth(key).send().await.and_then(|r| r.error_for_status()) {
        Ok(value) => value,
        Err(_) => return QuotaLine { provider: "GLM", value: "读取失败".into(), plan: None, resets: Vec::new() },
    };
    let payload: Value = match response.json().await { Ok(value) => value, Err(_) => return QuotaLine { provider: "GLM", value: "读取失败".into(), plan: None, resets: Vec::new() } };
    let mut limits: Vec<&Value> = payload.pointer("/data/limits").and_then(Value::as_array).into_iter().flatten()
        .filter(|item| item["type"].as_str() == Some("TOKENS_LIMIT")).collect();
    // 按窗口时长排序：unit 3 = 小时（number 个）、unit 6 = 周（number 个）。
    // 不能按 nextResetTime 排：未使用的窗口不返回该字段，会被错误地排到最后
    fn window_minutes(item: &Value) -> Option<i64> {
        let count = number(item.get("number"))?;
        match number(item.get("unit"))? {
            3 => Some(count * 60),
            6 => Some(count * 7 * 24 * 60),
            _ => None,
        }
    }
    let label = |item: &Value| match window_minutes(item) {
        Some(minutes) if minutes % 1_440 == 0 => format!("{}d", minutes / 1_440),
        Some(minutes) => format!("{}h", minutes / 60),
        None => "?".to_owned(),
    };
    limits.sort_by_key(|item| window_minutes(item).unwrap_or(i64::MAX));
    let pct = |item: &Value| format!("{}%", 100 - number(item.get("percentage")).unwrap_or(100));
    let value = match (limits.first(), limits.last()) {
        (Some(first), Some(last)) => format!("{} {} / {} {}", label(first), pct(first), label(last), pct(last)),
        _ => "暂无额度".into(),
    };
    let plan = payload.pointer("/data/level").and_then(Value::as_str).map(str::to_owned);
    // nextResetTime 为数字时间戳；未使用的窗口不返回该字段，直接跳过
    let resets = limits.iter().filter_map(|item| {
        Some(QuotaReset { label: label(item), resets_at_ms: reset_epoch_ms(item.get("nextResetTime"))? })
    }).collect();
    QuotaLine { provider: "GLM", value, plan, resets }
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
        return QuotaLine { provider: "KIMI", value: "未登录".into(), plan: None, resets: Vec::new() };
    };
    let mut payload = None;
    for path in paths {
        if let Some(value) = kimi_payload_from_path(client, &path).await {
            payload = Some(value);
            break;
        }
    }
    let Some(payload) = payload else {
        return QuotaLine { provider: "KIMI", value: "未登录".into(), plan: None, resets: Vec::new() };
    };
    let mut rows: Vec<(String, String, Option<i64>)> = vec![];
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
                rows.push((name, remaining(used, limit), reset_epoch_ms(detail.get("resetTime"))));
            }
        }
    }
    if let Some(usage) = payload["usage"].as_object() {
        if let Some(limit) = number(usage.get("limit")) {
            let used = number(usage.get("used")).or_else(|| number(usage.get("remaining")).map(|v| limit - v)).unwrap_or(0);
            rows.push(("7d".into(), remaining(used, limit), reset_epoch_ms(usage.get("resetTime"))));
        }
    }
    let five_hour = rows.iter().find(|(name, _, _)| name.contains("5h") || name.contains("5H")).or_else(|| rows.first());
    let seven_day = rows.iter().find(|(name, _, _)| name.contains("7d") || name.contains("7D")).or_else(|| rows.get(1));
    let value = match (five_hour, seven_day) {
        (Some((_, h5, _)), Some((_, d7, _))) => format!("5h {h5} / 7d {d7}"),
        (Some((name, pct, _)), None) => format!("{name} {pct}"),
        _ => "暂无额度".into(),
    };
    let resets = [five_hour, seven_day]
        .into_iter()
        .flatten()
        .filter_map(|(name, _, reset_at_ms)| {
            reset_at_ms.map(|ms| QuotaReset { label: name.clone(), resets_at_ms: ms })
        })
        .collect();
    let plan = payload.pointer("/user/membership/level").and_then(Value::as_str).map(kimi_membership_name);
    QuotaLine { provider: "KIMI", value, plan, resets }
}

async fn deepseek_line(client: &reqwest::Client, override_key: &str) -> QuotaLine {
    let Some(key) = provider_key_with_override("DeepSeek", override_key) else {
        return QuotaLine { provider: "DEEPSEEK", value: "未配置".into(), plan: None, resets: Vec::new() };
    };
    let response = match client.get("https://api.deepseek.com/user/balance")
        .bearer_auth(key).send().await.and_then(|r| r.error_for_status()) {
        Ok(value) => value,
        Err(_) => return QuotaLine { provider: "DEEPSEEK", value: "读取失败".into(), plan: None, resets: Vec::new() },
    };
    let payload: Value = match response.json().await { Ok(value) => value, Err(_) => return QuotaLine { provider: "DEEPSEEK", value: "读取失败".into(), plan: None, resets: Vec::new() } };
    let balance = payload["balance_infos"].as_array().and_then(|items| items.first())
        .and_then(|item| item["total_balance"].as_str()).unwrap_or("—");
    QuotaLine { provider: "DEEPSEEK", value: format!("余额 ¥{balance}"), plan: Some("Token".into()), resets: Vec::new() }
}

fn window_label(minutes: i64) -> String {
    if minutes % 1_440 == 0 {
        format!("{}d", minutes / 1_440)
    } else if minutes % 60 == 0 {
        format!("{}h", minutes / 60)
    } else {
        format!("{minutes}m")
    }
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

// 各接口的重置时间字段格式不一（GLM 数字时间戳、KIMI ISO 字符串、Codex 秒级时间戳或相对秒数），统一换算为 Unix 毫秒
fn reset_epoch_ms(value: Option<&Value>) -> Option<i64> {
    let value = value?;
    if let Some(num) = value.as_i64().or_else(|| value.as_str().and_then(|s| s.parse::<i64>().ok())) {
        return match num {
            // 毫秒级时间戳
            n if n >= 1_000_000_000_000 => Some(n),
            // 秒级时间戳；更小的值视为占位/无效
            n if n >= 1_000_000_000 => Some(n.saturating_mul(1000)),
            _ => None,
        };
    }
    value.as_str().and_then(parse_iso_utc_ms)
}

// 解析 "2026-08-27T01:58:06[.fff]Z" 形式的 UTC 时间为 Unix 毫秒（天数用民用历转换公式计算）
fn parse_iso_utc_ms(value: &str) -> Option<i64> {
    let rest = value.trim().strip_suffix('Z').unwrap_or(value.trim());
    let (date, time) = rest.split_once('T')?;
    let mut date_parts = date.split('-');
    let year: i64 = date_parts.next()?.parse().ok()?;
    let month: i64 = date_parts.next()?.parse().ok()?;
    let day: i64 = date_parts.next()?.parse().ok()?;
    let mut time_parts = time.split(':');
    let hour: i64 = time_parts.next()?.parse().ok()?;
    let minute: i64 = time_parts.next()?.parse().ok()?;
    let second_ms: i64 = (time_parts.next().unwrap_or("0").parse::<f64>().ok()? * 1000.0).round() as i64;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    // Days from civil：Howard Hinnant 的民用日期 → Unix 天数算法
    let shifted_year = if month <= 2 { year - 1 } else { year };
    let era = shifted_year.div_euclid(400);
    let year_of_era = shifted_year - era * 400;
    let month_of_year = (month + 9) % 12;
    let day_of_year = (153 * month_of_year + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;
    Some((days * 86_400 + hour * 3_600 + minute * 60) * 1_000 + second_ms)
}

// Codex 窗口的重置时间：优先用绝对时间 resetsAt（Unix 秒，新版 codex）；
// 旧版 codex 只有相对的 resetsInSeconds，按当前时间换算；两者都缺（接口未提供）时返回 None
fn codex_reset_epoch_ms(window: &Value) -> Option<i64> {
    let absolute = reset_epoch_ms(window.get("resetsAt").or_else(|| window.get("reset_at")));
    if absolute.is_some() {
        return absolute;
    }
    let in_secs = number(window.get("resetsInSeconds")).or_else(|| number(window.get("resets_in_seconds")))?;
    if in_secs <= 0 {
        return None;
    }
    Some(now_ms().saturating_add(in_secs.saturating_mul(1000)))
}

fn codex_window(window: &Value) -> Option<(i64, String, Option<i64>)> {
    let minutes = number(window.get("windowDurationMins"))?;
    let used = number(window.get("usedPercent")).unwrap_or(100).clamp(0, 100);
    Some((minutes, format!("{} {}%", window_label(minutes), 100 - used), codex_reset_epoch_ms(window)))
}

fn read_codex_limits(cli: &str) -> Option<(String, Option<String>, Vec<QuotaReset>)> {
    let mut command = Command::new(cli);
    command
        .args(["app-server", "--stdio"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = ManagedChild::spawn(&mut command).ok()?;

    let mut stdin = child.child.stdin.take()?;
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
        return None;
    }
    let stdout = child.child.stdout.take()?;
    let (sender, receiver) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let reader = BufReader::new(stdout);
        let mut value = None;
        for line in reader.lines().take(64) {
            let Ok(line) = line else { break };
            let Ok(payload) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if payload.get("id").and_then(Value::as_i64) != Some(2) {
                continue;
            }
            let Some(limits) = payload.pointer("/result/rateLimits") else {
                break;
            };
            let mut windows = ["secondary", "primary"]
                .into_iter()
                .filter_map(|name| limits.get(name).and_then(codex_window))
                .collect::<Vec<_>>();
            windows.sort_by_key(|(minutes, _, _)| *minutes);
            if !windows.is_empty() {
                let usage = windows
                    .iter()
                    .map(|(_, text, _)| text.as_str())
                    .collect::<Vec<_>>()
                    .join(" / ");
                let plan = limits
                    .get("planType")
                    .and_then(Value::as_str)
                    .map(readable_plan_name);
                let resets = windows
                    .iter()
                    .filter_map(|(minutes, _, reset_at_ms)| {
                        reset_at_ms.map(|ms| QuotaReset {
                            label: window_label(*minutes),
                            resets_at_ms: ms,
                        })
                    })
                    .collect();
                value = Some((usage, plan, resets));
            }
            break;
        }
        let _ = sender.send(value);
    });

    let value = receiver.recv_timeout(CODEX_CHILD_TIMEOUT).ok().flatten();
    child.terminate_and_wait();
    value
}

fn codex_line() -> QuotaLine {
    let cli = if std::path::Path::new("/Applications/ChatGPT.app/Contents/Resources/codex").is_file() {
        "/Applications/ChatGPT.app/Contents/Resources/codex"
    } else {
        "codex"
    };
    for attempt in 0..2 {
        if let Some((value, plan, resets)) = read_codex_limits(cli) {
            return QuotaLine { provider: "CODEX", value, plan, resets };
        }
        if attempt == 0 { std::thread::sleep(std::time::Duration::from_millis(600)); }
    }
    QuotaLine { provider: "CODEX", value: "读取失败".into(), plan: None, resets: Vec::new() }
}

#[tauri::command]
async fn get_quotas(app: tauri::AppHandle) -> Vec<QuotaLine> {
    let settings = read_settings(&app);
    let visible = |name: &str| settings.visible_providers.iter().any(|p| p == name);
    let client = app.state::<QuotaHttpClient>().0.clone();

    // 四个供应商互不依赖，并发拉取可把最坏刷新时长从超时之和降到单个最慢请求。
    let codex = async {
        if !visible("CODEX") { return None; }
        Some(tauri::async_runtime::spawn_blocking(codex_line).await.unwrap_or(QuotaLine {
            provider: "CODEX", value: "读取失败".into(), plan: None, resets: Vec::new()
        }))
    };
    let kimi = async {
        if visible("KIMI") { Some(kimi_line(&client).await) } else { None }
    };
    let glm = async {
        if visible("GLM") { Some(glm_line(&client, &settings.glm_api_key).await) } else { None }
    };
    let deepseek = async {
        if visible("DEEPSEEK") { Some(deepseek_line(&client, &settings.deepseek_api_key).await) } else { None }
    };
    let lines = futures_util::future::join4(codex, kimi, glm, deepseek).await;
    [lines.0, lines.1, lines.2, lines.3].into_iter().flatten().collect()
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

fn persist_settings(app: &tauri::AppHandle, settings: &BoardSettings) -> Result<(), String> {
    let path = settings_path(app)?;
    let parent = path.parent().ok_or_else(|| "设置保存位置无效".to_string())?;
    fs::create_dir_all(parent).map_err(|error| format!("无法创建设置目录：{error}"))?;
    let data = serde_json::to_vec_pretty(settings).map_err(|error| format!("无法序列化设置：{error}"))?;
    fs::write(path, data).map_err(|error| format!("无法保存设置：{error}"))
}

// 面板显隐直接由 Rust 控制，不依赖前端事件是否到达
fn apply_board_visibility(app: &tauri::AppHandle, show_board: bool) {
    if let Some(window) = app.get_webview_window("main") {
        if show_board {
            let _ = window.show();
        } else {
            let _ = window.hide();
        }
    }
}

#[tauri::command]
fn save_settings(app: tauri::AppHandle, settings: BoardSettings) -> Result<(), String> {
    let settings = settings.normalized();
    persist_settings(&app, &settings)?;
    refresh_tray(&app);
    apply_board_visibility(&app, settings.show_board);
    app.emit_to("main", "settings-updated", settings)
        .map_err(|error| format!("无法应用设置：{error}"))?;
    if let Some(window) = app.get_webview_window("settings") {
        window.close().map_err(|error| format!("无法关闭设置窗口：{error}"))?;
    }
    Ok(())
}

// ---------- 状态栏（托盘） ----------

#[derive(Clone, Deserialize, PartialEq, Eq)]
struct TrayQuotaLine {
    provider: String,
    value: String,
    /// 高峰期标识，由前端（拥有本机时区信息）按分钟计算后随刷新推送
    #[serde(default)]
    peak: bool,
}

#[derive(Default)]
struct TrayState {
    last_lines: Vec<TrayQuotaLine>,
    rendered_title: Option<String>,
    rendered_visible: Option<bool>,
}

// 状态栏文案：有百分比窗口按顺序拼接（前面 5h、后面 7d），
// 无百分比时取最后一个词（如 ¥7.38、读取失败），交给 macOS 原生文字渲染。
fn tray_title(provider: &str, value: &str) -> String {
    let provider = provider.to_uppercase();
    let percents: Vec<&str> = value
        .split(" / ")
        .filter_map(|part| part.rsplit_once(' ').map(|(_, tail)| tail))
        .filter(|tail| tail.ends_with('%'))
        .collect();
    if percents.is_empty() {
        let tail = value.rsplit_once(' ').map(|(_, tail)| tail).unwrap_or(value);
        format!("{provider} {tail}")
    } else {
        format!("{provider} {}", percents.join(" "))
    }
}

// 使用 macOS 原生菜单栏文字，避免为少量状态文本解析并常驻整套 CJK 字体轮廓。
// 高峰期在供应商名后缀 (高)（与看板一致）；失败/未配置状态不加，避免状态文案被稀释。
fn tray_display_title(provider: &str, value: &str, peak: bool) -> String {
    let has_value = !value.contains("失败") && !value.starts_with('未');
    let provider = if peak && has_value {
        format!("{}(高)", provider.to_uppercase())
    } else {
        provider.to_uppercase()
    };
    tray_title(&provider, value)
}

fn tray_menu(app: &tauri::AppHandle, settings: &BoardSettings) -> tauri::Result<Menu<tauri::Wry>> {
    let mut builder = MenuBuilder::new(app);
    for name in ALL_PROVIDERS
        .iter()
        .filter(|name| settings.visible_providers.iter().any(|visible| visible.as_str() == **name))
    {
        let item = CheckMenuItemBuilder::with_id(format!("tray-provider-{name}"), *name)
            .checked(settings.tray_provider == *name)
            .build(app)?;
        builder = builder.item(&item);
    }
    let version_item = MenuItemBuilder::with_id("tray-version", format!("v{}", app.package_info().version))
        .enabled(false)
        .build(app)?;
    let refresh_item = MenuItemBuilder::with_id("tray-refresh", "刷新").build(app)?;
    let settings_item = MenuItemBuilder::with_id("tray-settings", "设置").build(app)?;
    let update_item = MenuItemBuilder::with_id("tray-update", "检查更新").build(app)?;
    let quit_item = MenuItemBuilder::with_id("tray-quit", "退出").build(app)?;
    builder.separator().items(&[&version_item, &refresh_item, &settings_item, &update_item, &quit_item]).build()
}

fn refresh_tray_title(app: &tauri::AppHandle) {
    let settings = read_settings(app);
    let Some(tray) = app.tray_by_id("tray") else {
        return;
    };
    let (lines, visibility_changed) = {
        let tray_state = app.state::<Mutex<TrayState>>();
        let mut state = tray_state.lock().unwrap();
        let changed = state.rendered_visible != Some(settings.show_tray);
        state.rendered_visible = Some(settings.show_tray);
        (state.last_lines.clone(), changed)
    };
    if visibility_changed {
        let _ = tray.set_visible(settings.show_tray);
    }
    if !settings.show_tray {
        return;
    }
    let line = lines
        .iter()
        .find(|line| line.provider == settings.tray_provider)
        .or(lines.first());
    let title = line
        .map(|line| tray_display_title(&line.provider, &line.value, line.peak))
        .unwrap_or_else(|| "TOKEN".into());
    let changed = {
        let tray_state = app.state::<Mutex<TrayState>>();
        let mut state = tray_state.lock().unwrap();
        if state.rendered_title.as_deref() == Some(title.as_str()) {
            false
        } else {
            state.rendered_title = Some(title.clone());
            true
        }
    };
    if changed {
        let _ = tray.set_title(Some(title));
    }
}

// 设置变化后重建托盘菜单（供应商勾选列表可能变了）、应用显隐并刷新标题
fn refresh_tray(app: &tauri::AppHandle) {
    let settings = read_settings(app);
    if let Some(tray) = app.tray_by_id("tray") {
        if let Ok(menu) = tray_menu(app, &settings) {
            let _ = tray.set_menu(Some(menu));
        }
    }
    refresh_tray_title(app);
}

// 看板每次刷新额度后推送最新数据，托盘据此更新标题
#[tauri::command]
fn update_tray(app: tauri::AppHandle, lines: Vec<TrayQuotaLine>) {
    let changed = {
        let tray_state = app.state::<Mutex<TrayState>>();
        let mut state = tray_state.lock().unwrap();
        if state.last_lines == lines {
            false
        } else {
            state.last_lines = lines;
            true
        }
    };
    if changed {
        refresh_tray_title(&app);
    }
}

fn handle_tray_menu_event(app: &tauri::AppHandle, id: &str) {
    if let Some(provider) = id.strip_prefix("tray-provider-") {
        let settings = read_settings(app);
        let settings = BoardSettings { tray_provider: provider.to_owned(), ..settings }.normalized();
        let _ = persist_settings(app, &settings);
        refresh_tray(app);
        return;
    }
    match id {
        "tray-refresh" => {
            let _ = app.emit_to("main", "tray-refresh", ());
        }
        "tray-settings" => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let _ = open_settings(app).await;
            });
        }
        // 更新检查在前端执行（updater 插件的 JS API），转发给主窗口
        "tray-update" => {
            let _ = app.emit_to("main", "tray-check-updates", ());
        }
        "tray-quit" => app.exit(0),
        _ => {}
    }
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

fn active_dialogs() -> &'static Mutex<HashSet<&'static str>> {
    static DIALOGS: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();
    DIALOGS.get_or_init(|| Mutex::new(HashSet::new()))
}

fn spawn_managed_dialog(kind: &'static str, script: String) {
    {
        let mut dialogs = active_dialogs().lock().unwrap();
        if !dialogs.insert(kind) {
            return;
        }
    }

    let mut command = Command::new("osascript");
    command
        .args(["-e", &script])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    match ManagedChild::spawn(&mut command) {
        Ok(mut child) => {
            std::thread::spawn(move || {
                child.wait();
                active_dialogs().lock().unwrap().remove(kind);
            });
        }
        Err(_) => {
            active_dialogs().lock().unwrap().remove(kind);
        }
    }
}

// CODEX 重置提醒同类弹窗只保留一个；后台线程在用户关闭后 wait 回收 osascript。
#[tauri::command]
fn notify_codex_full() {
    spawn_managed_dialog(
        "codex-reset",
        r#"display dialog "codex重置了!!!老铁,抓紧蹬!!" with title "Token 看板" buttons {"知道了"} default button "知道了""#.into(),
    );
}

// 更新安装失败时弹系统对话框，引导用户手动解除 quarantine。
fn update_failed_script() -> String {
    let lines = [
        "更新失败,可能是因为安装包没有进行权限处理,请按以下步骤尝试:",
        "1.关闭app并将Token 看板.app移入Applications中",
        r#"2.终端执行xattr -dr com.apple.quarantine \"/Applications/Token 看板.app\",没有报错就是成功了"#,
        "3.重新打开app会自动更新下载",
    ];
    let message = lines.join("\" & return & \"");
    format!("display dialog \"{message}\" with title \"Token 看板\" buttons {{\"知道了\"}} default button \"知道了\"")
}

#[tauri::command]
fn notify_update_failed() {
    spawn_managed_dialog("update-failed", update_failed_script());
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
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .manage(QuotaHttpClient::default())
        .manage(Mutex::new(TrayState::default()))
        .setup(|app| {
            let settings = read_settings(app.handle());
            let menu = tray_menu(app.handle(), &settings)?;
            TrayIconBuilder::with_id("tray")
                .title("TOKEN")
                .menu(&menu)
                .on_menu_event(|tray, event| handle_tray_menu_event(tray.app_handle(), event.id().as_ref()))
                .build(app)?;
            if let Some(tray) = app.tray_by_id("tray") {
                let _ = tray.set_visible(settings.show_tray);
            }
            apply_board_visibility(app.handle(), settings.show_board);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            open_app,
            get_quotas,
            get_settings,
            save_settings,
            open_settings,
            close_settings,
            close_app,
            notify_codex_full,
            notify_update_failed,
            update_tray
        ])
        .build(tauri::generate_context!())
        .expect("构建 Token 看板失败");

    app.run(|_, event| {
        if matches!(event, tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit) {
            terminate_active_children();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn managed_child_drop_terminates_and_reaps_process() {
        let mut command = Command::new("/bin/sleep");
        command
            .arg("30")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let child = ManagedChild::spawn(&mut command).expect("sleep child should start");
        let pid = child.pid;
        drop(child);

        let mut status = 0;
        let result = unsafe { libc::waitpid(pid as i32, &mut status, libc::WNOHANG) };
        assert_eq!(result, -1, "child must already be reaped");
        assert!(!active_child_pids().lock().unwrap().contains(&pid));
    }

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
        assert!(settings.auto_update);
        assert!(settings.show_resets);
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
    fn normalized_settings_keep_at_least_one_display_target() {
        let settings = BoardSettings { show_board: false, show_tray: false, ..BoardSettings::default() }.normalized();
        assert!(settings.show_board);
        let settings = BoardSettings { show_board: false, show_tray: true, ..BoardSettings::default() }.normalized();
        assert!(!settings.show_board);
        assert!(settings.show_tray);
    }

    #[test]
    fn tray_title_joins_percent_windows_and_falls_back_to_last_word() {
        assert_eq!(tray_title("CODEX", "7d 97%"), "CODEX 97%");
        assert_eq!(tray_title("KIMI", "5h 85% / 7d 85%"), "KIMI 85% 85%");
        assert_eq!(tray_title("DEEPSEEK", "余额 ¥7.38"), "DEEPSEEK ¥7.38");
        assert_eq!(tray_title("CODEX", "读取失败"), "CODEX 读取失败");
    }

    #[test]
    fn tray_display_title_uses_native_text_and_peak_marker() {
        assert_eq!(
            tray_display_title("GLM", "5h 41% / 7d 86%", true),
            "GLM(高) 41% 86%"
        );
        assert_eq!(
            tray_display_title("DEEPSEEK", "余额 ¥14.99", true),
            "DEEPSEEK(高) ¥14.99"
        );
        // 失败/未配置状态即使处于高峰期也不加标识。
        assert_eq!(tray_display_title("GLM", "读取失败", true), "GLM 读取失败");
        assert_eq!(tray_display_title("GLM", "未配置", true), "GLM 未配置");
    }

    #[test]
    fn update_failed_script_is_valid_multiline_applescript_dialog() {
        let script = update_failed_script();
        assert!(script.starts_with("display dialog \""));
        assert!(script.contains("\" & return & \""));
        // 路径两侧的引号必须转义，否则 AppleScript 编译失败
        assert!(script.contains(r#"com.apple.quarantine \"/Applications/Token 看板.app\""#));
        assert!(script.contains("1.关闭app并将Token 看板.app移入Applications中"));
        assert!(script.contains("3.重新打开app会自动更新下载"));
    }

    #[test]
    fn provider_plan_names_are_readable() {
        assert_eq!(readable_plan_name("plus"), "Plus");
        assert_eq!(readable_plan_name("enterprise_team"), "Enterprise Team");
        assert_eq!(kimi_membership_name("LEVEL_INTERMEDIATE"), "Allegretto");
        assert_eq!(kimi_membership_name("LEVEL_PREMIUM"), "Vivace");
    }

    #[test]
    fn codex_reset_time_prefers_absolute_timestamp_and_falls_back_to_relative() {
        // 新版 codex：resetsAt 为 Unix 秒，换算成毫秒透传给前端
        let absolute = serde_json::json!({ "windowDurationMins": 300, "usedPercent": 10, "resetsAt": 1_700_000_000_i64 });
        assert_eq!(codex_reset_epoch_ms(&absolute), Some(1_700_000_000_000));
        // 旧版 codex：只有相对秒数，换算为「当前时间 + 秒数」
        let relative = serde_json::json!({ "resetsInSeconds": 120 });
        let got = codex_reset_epoch_ms(&relative).expect("relative reset should resolve");
        let base = now_ms();
        assert!((base..=base + 120_000).contains(&got), "dbg got={got} base={base}");
        // 占位值与缺失字段都不产生重置时间
        assert_eq!(codex_reset_epoch_ms(&serde_json::json!({ "resetsAt": 0 })), None);
        assert_eq!(codex_reset_epoch_ms(&serde_json::json!({})), None);
    }

    #[test]
    fn codex_window_carries_label_and_reset_time() {
        let window = serde_json::json!({ "windowDurationMins": 10_080, "usedPercent": 40, "resetsAt": 1_700_000_000_i64 });
        let (minutes, text, reset_at_ms) = codex_window(&window).expect("7d window");
        assert_eq!(minutes, 10_080);
        assert_eq!(text, "7d 60%");
        assert_eq!(reset_at_ms, Some(1_700_000_000_000));
        let plain = serde_json::json!({ "windowDurationMins": 300, "usedPercent": 20 });
        let (_, text, reset_at_ms) = codex_window(&plain).expect("5h window");
        assert_eq!(text, "5h 80%");
        assert_eq!(reset_at_ms, None);
    }

    #[test]
    fn reset_epoch_ms_handles_timestamps_and_iso_strings() {
        // GLM：数字毫秒 / 秒时间戳
        assert_eq!(reset_epoch_ms(Some(&serde_json::json!(1_770_000_000_000_i64))), Some(1_770_000_000_000));
        assert_eq!(reset_epoch_ms(Some(&serde_json::json!("1770000000"))), Some(1_770_000_000_000));
        // KIMI：ISO 8601 UTC 字符串（含小数秒）
        let iso: Value = "2026-08-27T01:58:06Z".into();
        assert_eq!(reset_epoch_ms(Some(&iso)), Some(1_787_795_886_000));
        let iso_fractional: Value = "2026-08-27T01:58:06.064554Z".into();
        assert_eq!(reset_epoch_ms(Some(&iso_fractional)), Some(1_787_795_886_065));
        // 占位值与垃圾输入返回 None
        assert_eq!(reset_epoch_ms(Some(&serde_json::json!(0))), None);
        assert_eq!(reset_epoch_ms(Some(&serde_json::json!("not-a-time"))), None);
        assert_eq!(reset_epoch_ms(None), None);
        // 跨月边界（闰年）也正确
        let leap: Value = "2024-02-29T23:59:59Z".into();
        assert_eq!(reset_epoch_ms(Some(&leap)), Some(1_709_251_199_000));
    }
}
