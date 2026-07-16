use std::{
    ffi::c_void,
    process::Command,
    ptr,
    sync::mpsc::{self, SyncSender},
    thread,
    time::Duration,
};

use core_foundation::{
    base::{CFGetTypeID, CFRelease, CFTypeRef, TCFType},
    string::{CFString, CFStringRef},
};

type AXUIElementRef = *mut c_void;
type AXError = i32;
const AX_SUCCESS: AXError = 0;
const CG_SESSION_EVENT_TAP: u32 = 1;
const CG_HEAD_INSERT_EVENT_TAP: u32 = 0;
const CG_EVENT_TAP_DEFAULT: u32 = 0;
const CG_EVENT_LEFT_MOUSE_UP: u32 = 2;
const CG_EVENT_TAP_DISABLED_BY_TIMEOUT: u32 = u32::MAX - 1;
const CG_EVENT_TAP_DISABLED_BY_USER_INPUT: u32 = u32::MAX;

type CFMachPortRef = *mut c_void;
type CFRunLoopRef = *mut c_void;
type CFRunLoopSourceRef = *mut c_void;
type CGEventTapProxy = *mut c_void;
type CGEventRef = *mut c_void;
type MouseReleaseCallback = Box<dyn Fn() + Send + 'static>;

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> bool;
    fn AXUIElementCreateSystemWide() -> AXUIElementRef;
    fn AXUIElementCopyAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: *mut CFTypeRef,
    ) -> AXError;
}

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn CGEventTapCreate(
        tap: u32,
        place: u32,
        options: u32,
        events_of_interest: u64,
        callback: unsafe extern "C" fn(CGEventTapProxy, u32, CGEventRef, *mut c_void) -> CGEventRef,
        user_info: *mut c_void,
    ) -> CFMachPortRef;
    fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    static kCFRunLoopCommonModes: CFStringRef;
    fn CFMachPortCreateRunLoopSource(
        allocator: *const c_void,
        port: CFMachPortRef,
        order: isize,
    ) -> CFRunLoopSourceRef;
    fn CFRunLoopGetCurrent() -> CFRunLoopRef;
    fn CFRunLoopAddSource(run_loop: CFRunLoopRef, source: CFRunLoopSourceRef, mode: CFStringRef);
    fn CFRunLoopRun() -> i32;
}

pub fn is_accessibility_trusted() -> bool {
    unsafe { AXIsProcessTrusted() }
}

pub fn request_accessibility() -> Result<(), String> {
    Command::new("open")
        .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("无法打开辅助功能设置：{error}"))
}

/// Unlike rdev's macOS listener, this tap only receives mouse-up events and never
/// asks Text Services to translate keyboard input from a non-main thread.
pub fn start_mouse_release_listener(callback: impl Fn() + Send + 'static) -> Result<(), String> {
    let (ready_tx, ready_rx) = mpsc::sync_channel(1);
    thread::Builder::new()
        .name("xuandu-mouse-listener".to_owned())
        .spawn(move || run_mouse_event_tap(callback, ready_tx))
        .map_err(|error| format!("无法创建鼠标监听线程：{error}"))?;

    ready_rx
        .recv_timeout(Duration::from_secs(2))
        .map_err(|_| "初始化鼠标监听超时。".to_owned())?
}

fn run_mouse_event_tap(
    callback: impl Fn() + Send + 'static,
    ready: SyncSender<Result<(), String>>,
) {
    let callback: Box<MouseReleaseCallback> = Box::new(Box::new(callback));
    let callback_ptr = Box::into_raw(callback) as *mut c_void;

    unsafe {
        let tap = CGEventTapCreate(
            CG_SESSION_EVENT_TAP,
            CG_HEAD_INSERT_EVENT_TAP,
            CG_EVENT_TAP_DEFAULT,
            1_u64 << CG_EVENT_LEFT_MOUSE_UP,
            mouse_event_callback,
            callback_ptr,
        );
        if tap.is_null() {
            drop(Box::from_raw(callback_ptr as *mut MouseReleaseCallback));
            let _ = ready.send(Err(
                "无法创建全局鼠标监听。请确认已授予辅助功能权限。".to_owned()
            ));
            return;
        }

        let source = CFMachPortCreateRunLoopSource(ptr::null(), tap, 0);
        if source.is_null() {
            CFRelease(tap as CFTypeRef);
            drop(Box::from_raw(callback_ptr as *mut MouseReleaseCallback));
            let _ = ready.send(Err("无法将鼠标监听加入系统运行循环。".to_owned()));
            return;
        }

        CFRunLoopAddSource(CFRunLoopGetCurrent(), source, kCFRunLoopCommonModes);
        CGEventTapEnable(tap, true);
        if ready.send(Ok(())).is_err() {
            CFRelease(source as CFTypeRef);
            CFRelease(tap as CFTypeRef);
            drop(Box::from_raw(callback_ptr as *mut MouseReleaseCallback));
            return;
        }

        CFRunLoopRun();
        CFRelease(source as CFTypeRef);
        CFRelease(tap as CFTypeRef);
        drop(Box::from_raw(callback_ptr as *mut MouseReleaseCallback));
    }
}

unsafe extern "C" fn mouse_event_callback(
    proxy: CGEventTapProxy,
    event_type: u32,
    event: CGEventRef,
    user_info: *mut c_void,
) -> CGEventRef {
    if event_type == CG_EVENT_LEFT_MOUSE_UP {
        let callback = &*(user_info as *const MouseReleaseCallback);
        callback();
    } else if event_type == CG_EVENT_TAP_DISABLED_BY_TIMEOUT
        || event_type == CG_EVENT_TAP_DISABLED_BY_USER_INPUT
    {
        CGEventTapEnable(proxy as CFMachPortRef, true);
    }
    event
}

pub fn selected_text() -> Result<Option<String>, String> {
    if !is_accessibility_trusted() {
        return Ok(None);
    }

    unsafe {
        let system = AXUIElementCreateSystemWide();
        if system.is_null() {
            return Ok(None);
        }

        let focused_attribute = CFString::new("AXFocusedUIElement");
        let selected_attribute = CFString::new("AXSelectedText");
        let mut focused: CFTypeRef = ptr::null();
        let focused_result = AXUIElementCopyAttributeValue(
            system,
            focused_attribute.as_concrete_TypeRef(),
            &mut focused,
        );

        if focused_result != AX_SUCCESS || focused.is_null() {
            CFRelease(system as CFTypeRef);
            return Ok(None);
        }

        let mut selected: CFTypeRef = ptr::null();
        let selected_result = AXUIElementCopyAttributeValue(
            focused as AXUIElementRef,
            selected_attribute.as_concrete_TypeRef(),
            &mut selected,
        );
        CFRelease(focused);
        CFRelease(system as CFTypeRef);

        if selected_result != AX_SUCCESS || selected.is_null() {
            return Ok(None);
        }
        if CFGetTypeID(selected) != CFString::type_id() {
            CFRelease(selected);
            return Ok(None);
        }

        let string = CFString::wrap_under_create_rule(selected as CFStringRef);
        Ok(Some(string.to_string()))
    }
}
