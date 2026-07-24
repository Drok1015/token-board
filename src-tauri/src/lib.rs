use serde::Serialize;
use serde_json::Value;
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    process::{Command, Stdio},
};

#[derive(Serialize)]
struct QuotaLine {
    provider: &'static str,
    value: String,
}

fn home_file(path: &str) -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(path))
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
        return QuotaLine { provider: "GLM", value: "未配置".into() };
    };
    let response = match client.get("https://open.bigmodel.cn/api/monitor/usage/quota/limit")
        .bearer_auth(key).send().await.and_then(|r| r.error_for_status()) {
        Ok(value) => value,
        Err(_) => return QuotaLine { provider: "GLM", value: "读取失败".into() },
    };
    let payload: Value = match response.json().await { Ok(value) => value, Err(_) => return QuotaLine { provider: "GLM", value: "读取失败".into() } };
    let mut limits: Vec<&Value> = payload.pointer("/data/limits").and_then(Value::as_array).into_iter().flatten()
        .filter(|item| item["type"].as_str() == Some("TOKENS_LIMIT")).collect();
    limits.sort_by_key(|item| number(item.get("nextResetTime")).unwrap_or(i64::MAX));
    let pct = |item: &Value| format!("{}%", 100 - number(item.get("percentage")).unwrap_or(100));
    match (limits.first(), limits.last()) {
        (Some(first), Some(last)) => QuotaLine { provider: "GLM", value: format!("5h {} / 7d {}", pct(first), pct(last)) },
        _ => QuotaLine { provider: "GLM", value: "暂无额度".into() },
    }
}

async fn kimi_line(client: &reqwest::Client) -> QuotaLine {
    let Some(path) = home_file(".kimi/credentials/kimi-code.json") else {
        return QuotaLine { provider: "KIMI", value: "未登录".into() };
    };
    let credential: Value = match fs::read(path).ok().and_then(|data| serde_json::from_slice(&data).ok()) {
        Some(value) => value,
        None => return QuotaLine { provider: "KIMI", value: "未登录".into() },
    };
    let token = credential["access_token"].as_str().unwrap_or_default();
    let response = match client.get("https://api.kimi.com/coding/v1/usages")
        .bearer_auth(token).send().await.and_then(|r| r.error_for_status()) {
        Ok(value) => value,
        Err(_) => return QuotaLine { provider: "KIMI", value: "认证失效".into() },
    };
    let payload: Value = match response.json().await { Ok(value) => value, Err(_) => return QuotaLine { provider: "KIMI", value: "读取失败".into() } };
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
        (Some((_, h5)), Some((_, d7))) => QuotaLine { provider: "KIMI", value: format!("5h {h5} / 7d {d7}") },
        (Some((name, pct)), None) => QuotaLine { provider: "KIMI", value: format!("{name} {pct}") },
        _ => QuotaLine { provider: "KIMI", value: "暂无额度".into() },
    }
}

async fn deepseek_line(client: &reqwest::Client) -> QuotaLine {
    let Some(key) = provider_key("DeepSeek") else {
        return QuotaLine { provider: "DEEPSEEK", value: "未配置".into() };
    };
    let response = match client.get("https://api.deepseek.com/user/balance")
        .bearer_auth(key).send().await.and_then(|r| r.error_for_status()) {
        Ok(value) => value,
        Err(_) => return QuotaLine { provider: "DEEPSEEK", value: "读取失败".into() },
    };
    let payload: Value = match response.json().await { Ok(value) => value, Err(_) => return QuotaLine { provider: "DEEPSEEK", value: "读取失败".into() } };
    let balance = payload["balance_infos"].as_array().and_then(|items| items.first())
        .and_then(|item| item["total_balance"].as_str()).unwrap_or("—");
    QuotaLine { provider: "DEEPSEEK", value: format!("余额 ¥{balance}") }
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

fn codex_line() -> QuotaLine {
    let cli = if std::path::Path::new("/Applications/ChatGPT.app/Contents/Resources/codex").is_file() {
        "/Applications/ChatGPT.app/Contents/Resources/codex"
    } else {
        "codex"
    };
    let mut child = match Command::new(cli)
        .args(["app-server", "--stdio"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => return QuotaLine { provider: "CODEX", value: "未安装".into() },
    };

    let Some(mut stdin) = child.stdin.take() else {
        return QuotaLine { provider: "CODEX", value: "读取失败".into() };
    };
    let initialize = serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": { "clientInfo": { "name": "额度脉搏", "version": "0.1.0" }, "capabilities": { "experimentalApi": true } }
    });
    let read_limits = serde_json::json!({
        "jsonrpc": "2.0", "id": 2, "method": "account/rateLimits/read", "params": Value::Null
    });
    if writeln!(stdin, "{initialize}").is_err()
        || writeln!(stdin, "{read_limits}").is_err()
        || stdin.flush().is_err()
    {
        return QuotaLine { provider: "CODEX", value: "读取失败".into() };
    }

    let Some(stdout) = child.stdout.take() else {
        return QuotaLine { provider: "CODEX", value: "读取失败".into() };
    };
    let reader = BufReader::new(stdout);
    let mut value = "读取失败".to_string();
    for line in reader.lines().take(12) {
        let Ok(line) = line else { break };
        let Ok(payload) = serde_json::from_str::<Value>(&line) else { continue };
        if payload.get("id").and_then(Value::as_i64) != Some(2) { continue; }
        let Some(limits) = payload.pointer("/result/rateLimits") else { break };
        let mut windows = ["secondary", "primary"].into_iter()
            .filter_map(|name| limits.get(name).and_then(codex_window))
            .collect::<Vec<_>>();
        windows.sort_by_key(|(minutes, _)| *minutes);
        if !windows.is_empty() {
            value = windows.into_iter().map(|(_, text)| text).collect::<Vec<_>>().join(" / ");
        }
        break;
    }
    let _ = child.kill();
    QuotaLine { provider: "CODEX", value }
}

#[tauri::command]
async fn get_quotas() -> Vec<QuotaLine> {
    let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(10)).build();
    let Ok(client) = client else { return vec![] };
    let codex = tauri::async_runtime::spawn_blocking(codex_line);
    let mut quotas = vec![kimi_line(&client).await, glm_line(&client).await, deepseek_line(&client).await];
    let codex = codex.await.unwrap_or(QuotaLine { provider: "CODEX", value: "读取失败".into() });
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
        .expect("启动汇兑小猪失败");
}
