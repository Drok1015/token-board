#[tauri::command]
fn open_app(app: &str) -> Result<(), String> {
    let app_name = match app {
        "huide" => "汇兑",
        "renren" => "人人视频 for Mac",
        "parallels" => "Parallels Desktop",
        _ => return Err("不允许打开未配置的应用".into()),
    };

    let status = std::process::Command::new("open")
        .args(["-a", app_name])
        .status()
        .map_err(|error| format!("无法调用 macOS open 命令：{error}"))?;

    if status.success() { Ok(()) } else { Err(format!("未能打开 {app_name}")) }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![open_app])
        .run(tauri::generate_context!())
        .expect("启动汇兑小猪失败");
}
