pub fn tr(key: &str, values: &[(&str, String)]) -> String {
    let zh = is_zh_locale();
    let template = match (zh, key) {
        (true, "pick_admin_files") => "选择要共享的文件",
        (false, "pick_admin_files") => "Share files",
        (true, "save_file") => "保存文件",
        (false, "save_file") => "Save file",
        (true, "source_missing") => "源文件不存在",
        (false, "source_missing") => "File missing",
        (true, "source_missing_gone") => "源文件已不存在",
        (false, "source_missing_gone") => "File missing",
        (true, "not_supported") => "当前系统不支持打开文件位置",
        (false, "not_supported") => "Cannot open file location",
        (true, "invalid_port") => "端口号无效",
        (false, "invalid_port") => "Invalid port",
        (true, "update_failed") => "检查更新失败",
        (false, "update_failed") => "Update failed",
        (true, "check_update") => "检查更新",
        (false, "check_update") => "Check updates",
        (true, "latest_version") => "当前已经是最新版本。",
        (false, "latest_version") => "Already up to date.",
        (true, "new_version") => "发现新版本",
        (false, "new_version") => "Update available",
        (true, "about_title") => "关于 FileShare",
        (false, "about_title") => "About FileShare",
        (true, "about_desc") => "FileShare {version}\n局域网文件共享工具\n\n作者: Trifolium Wang\nGitHub: https://github.com/tri5m/file-share",
        (false, "about_desc") => "FileShare {version}\nLAN file sharing\n\nAuthor: Trifolium Wang\nGitHub: https://github.com/tri5m/file-share",
        (true, "stop_share") => "停止分享",
        (false, "stop_share") => "STOP",
        (true, "start_share") => "启动分享",
        (false, "start_share") => "Start",
        (true, "checking") => "检查中...",
        (false, "checking") => "Checking...",
        (true, "range_invalid") => "请求范围无效，文件大小为 {file_size} 字节",
        (false, "range_invalid") => "Invalid range. Size: {file_size} bytes",
        (true, "server_not_started") => "服务未启动",
        (false, "server_not_started") => "Not started",
        (true, "empty_text") => "文本不能为空",
        (false, "empty_text") => "Text is empty",
        (true, "admin_picker_required") => "管理端共享文件请使用系统文件选择器",
        (false, "admin_picker_required") => "Use the system file picker",
        (true, "please_select_file") => "请选择文件",
        (false, "please_select_file") => "Select a file",
        (true, "selected_file_missing") => "所选文件不存在",
        (false, "selected_file_missing") => "File missing",
        (true, "only_file") => "只能共享文件",
        (false, "only_file") => "Files only",
        (true, "paste_file_too_large") => "无本机路径的粘贴文件最大支持 {size}，请改用拖拽或选择文件",
        (false, "paste_file_too_large") => "Pasted files without a local path are limited to {size}. Please drag or choose the file instead.",
        (true, "item_missing") => "条目不存在",
        (false, "item_missing") => "Not found",
        (true, "text_no_download") => "文本无需下载",
        (false, "text_no_download") => "Text has no download",
        (true, "forbidden") => "管理端只能在服务器本机访问",
        (false, "forbidden") => "Local access only",
        (true, "text_snippet") => "文本片段",
        (false, "text_snippet") => "Text snippet",
        (true, "port_taken") => "端口 {port} 已被占用，请换一个端口或关闭占用该端口的程序",
        (false, "port_taken") => "Port {port} is in use.",
        (true, "port_failed") => "端口 {port} 启动失败：{error}",
        (false, "port_failed") => "Port {port} failed: {error}",
        _ => key,
    };

    values.iter().fold(template.to_string(), |acc, (name, value)| {
        acc.replace(&format!("{{{name}}}"), value)
    })
}

fn is_zh_locale() -> bool {
    std::env::var("LANG")
        .or_else(|_| std::env::var("LC_ALL"))
        .or_else(|_| std::env::var("LC_MESSAGES"))
        .unwrap_or_default()
        .to_lowercase()
        .starts_with("zh")
}
