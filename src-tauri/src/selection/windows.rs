use windows::{
    Win32::{
        Foundation::POINT,
        System::Com::{
            CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
        },
        UI::{
            Accessibility::{
                CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationTextPattern,
                UIA_TextPatternId,
            },
            WindowsAndMessaging::{GetCursorPos, GetForegroundWindow},
        },
    },
};

pub fn start_mouse_release_listener(callback: impl Fn() + Send + 'static) -> Result<(), String> {
    std::thread::Builder::new()
        .name("xuandu-mouse-listener".to_owned())
        .spawn(move || {
            let mut cursor = (0.0_f64, 0.0_f64);
            let mut press_origin = None;
            let mut dragged = false;
            let mut last_click: Option<(std::time::Instant, f64, f64)> = None;
            let _ = rdev::listen(move |event| {
                match event.event_type {
                    rdev::EventType::MouseMove { x, y } => {
                        cursor = (x, y);
                        if let Some((start_x, start_y)) = press_origin {
                            let dx: f64 = x - start_x;
                            let dy: f64 = y - start_y;
                            if dx * dx + dy * dy >= 16.0_f64 {
                                dragged = true;
                            }
                        }
                    }
                    rdev::EventType::ButtonPress(rdev::Button::Left) => {
                        press_origin = Some(cursor);
                        dragged = false;
                    }
                    rdev::EventType::ButtonRelease(rdev::Button::Left) => {
                        let now = std::time::Instant::now();
                        let double_click = last_click.is_some_and(|(at, x, y)| {
                            let dx = cursor.0 - x;
                            let dy = cursor.1 - y;
                            now.duration_since(at) <= std::time::Duration::from_millis(500)
                                && dx * dx + dy * dy <= 25.0
                        });
                        if dragged || double_click {
                            callback();
                        }
                        last_click = Some((now, cursor.0, cursor.1));
                        press_origin = None;
                        dragged = false;
                    }
                    _ => {}
                }
            });
        })
        .map(|_| ())
        .map_err(|error| format!("创建鼠标监听线程失败：{error}"))
}

pub fn is_accessibility_trusted() -> bool {
    true
}

pub fn request_accessibility() -> Result<(), String> {
    Ok(())
}

pub fn selected_text() -> Result<Option<String>, String> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let hwnd = GetForegroundWindow();
        if hwnd.0.is_null() {
            return Ok(None);
        }

        let automation: IUIAutomation =
            CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)
                .map_err(|error| format!("初始化 Windows UI Automation 失败：{error}"))?;

        // Browser and editor top-level windows rarely expose TextPattern directly.
        // Check the focused control and pointer target, then walk up their UIA parents.
        if let Ok(element) = automation.GetFocusedElement() {
            if let Some(text) = selected_text_from_element(&automation, element) {
                return Ok(Some(text));
            }
        }

        let mut point = POINT::default();
        if GetCursorPos(&mut point).is_ok() {
            if let Ok(element) = automation.ElementFromPoint(point) {
                if let Some(text) = selected_text_from_element(&automation, element) {
                    return Ok(Some(text));
                }
            }
        }

        if let Ok(element) = automation.ElementFromHandle(hwnd) {
            if let Some(text) = selected_text_from_element(&automation, element) {
                return Ok(Some(text));
            }
        }

        Ok(None)
    }
}

unsafe fn selected_text_from_element(
    automation: &IUIAutomation,
    mut element: IUIAutomationElement,
) -> Option<String> {
    let walker = automation.ControlViewWalker().ok()?;
    for _ in 0..12 {
        if let Ok(pattern) =
            element.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId)
        {
            if let Ok(ranges) = pattern.GetSelection() {
                let length = ranges.Length().unwrap_or(0);
                for index in 0..length {
                    if let Ok(range) = ranges.GetElement(index) {
                        if let Ok(value) = range.GetText(-1) {
                            let text = value.to_string();
                            if !text.trim().is_empty() {
                                return Some(text);
                            }
                        }
                    }
                }
            }
        }
        element = walker.GetParentElement(&element).ok()?;
    }
    None
}
