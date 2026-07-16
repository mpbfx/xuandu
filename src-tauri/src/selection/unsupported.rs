pub fn is_accessibility_trusted() -> bool {
    false
}

pub fn request_accessibility() -> Result<(), String> {
    Err("当前平台不支持跨应用选区读取。".to_owned())
}

pub fn start_mouse_release_listener(_callback: impl Fn() + Send + 'static) -> Result<(), String> {
    Err("当前平台不支持跨应用鼠标选区监听。".to_owned())
}

pub fn selected_text() -> Result<Option<String>, String> {
    Ok(None)
}
