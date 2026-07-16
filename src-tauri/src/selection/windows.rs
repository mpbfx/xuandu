use windows::{
    core::Interface,
    Win32::{
        System::Com::{
            CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
        },
        UI::{
            Accessibility::{
                CUIAutomation, IUIAutomation, IUIAutomationTextPattern, UIA_TextPatternId,
            },
            WindowsAndMessaging::GetForegroundWindow,
        },
    },
};

pub fn start_mouse_release_listener(callback: impl Fn() + Send + 'static) -> Result<(), String> {
    std::thread::Builder::new()
        .name("xuandu-mouse-listener".to_owned())
        .spawn(move || {
            let _ = rdev::listen(move |event| {
                if matches!(
                    event.event_type,
                    rdev::EventType::ButtonRelease(rdev::Button::Left)
                ) {
                    callback();
                }
            });
        })
        .map(|_| ())
        .map_err(|error| format!("无法创建鼠标监听线程：{error}"))
}

pub fn is_accessibility_trusted() -> bool {
    true
}

pub fn request_accessibility() -> Result<(), String> {
    Ok(())
}

pub fn selected_text() -> Result<Option<String>, String> {
    unsafe {
        // UI Automation can be initialized repeatedly on the input listener thread.
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let hwnd = GetForegroundWindow();
        if hwnd.0 == 0 {
            return Ok(None);
        }

        let automation: IUIAutomation =
            CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)
                .map_err(|error| format!("无法初始化 Windows UI Automation：{error}"))?;
        let element = automation
            .ElementFromHandle(hwnd)
            .map_err(|_| "前台窗口未提供可访问文本。".to_owned())?;
        let pattern: IUIAutomationTextPattern = element
            .GetCurrentPatternAs(UIA_TextPatternId)
            .map_err(|_| "当前控件未提供选区。".to_owned())?;
        let ranges = pattern
            .GetSelection()
            .map_err(|_| "当前控件未提供选区。".to_owned())?;

        if ranges.Length().unwrap_or(0) == 0 {
            return Ok(None);
        }
        let range = ranges
            .GetElement(0)
            .map_err(|_| "无法读取当前选区。".to_owned())?;
        let value = range
            .GetText(-1)
            .map_err(|_| "无法读取当前选区。".to_owned())?;

        Ok((!value.is_empty()).then_some(value.to_string()))
    }
}
