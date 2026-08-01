use std::collections::VecDeque;
use std::ffi::c_void;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use accessibility_sys_ng::{
    AXError, AXIsProcessTrusted, AXObserverAddNotification, AXObserverCreate,
    AXObserverGetRunLoopSource, AXObserverRef, AXObserverRemoveNotification,
    AXUIElementCopyAttributeValue, AXUIElementCreateApplication, AXUIElementGetTypeID,
    AXUIElementRef, AXUIElementSetMessagingTimeout, kAXErrorCannotComplete,
    kAXErrorNotificationAlreadyRegistered, kAXErrorNotificationUnsupported,
    kAXFocusedUIElementChangedNotification, kAXFocusedWindowChangedNotification,
    kAXMainWindowChangedNotification, kAXSelectedChildrenChangedNotification,
    kAXTitleChangedNotification, kAXValueChangedNotification, kAXWindowCreatedNotification,
};
use core_foundation::array::{CFArrayGetCount, CFArrayGetValueAtIndex, CFArrayRef};
use core_foundation::base::{
    CFEqual, CFGetTypeID, CFRelease, CFRetain, CFType, CFTypeRef, TCFType,
};
use core_foundation::dictionary::{CFDictionaryGetValueIfPresent, CFDictionaryRef};
use core_foundation::number::{CFNumberGetValue, CFNumberRef, kCFNumberSInt32Type};
use core_foundation::runloop::{
    CFRunLoopAddSource, CFRunLoopGetCurrent, CFRunLoopRemoveSource, kCFRunLoopDefaultMode,
};
use core_foundation::string::{CFString, CFStringRef};
use core_foundation::url::CFURL;
use core_graphics::window::{
    CGWindowListCopyWindowInfo, kCGNullWindowID, kCGWindowLayer,
    kCGWindowListExcludeDesktopElements, kCGWindowListOptionOnScreenOnly, kCGWindowName,
    kCGWindowNumber, kCGWindowOwnerPID,
};
use tracing::{debug, warn};

use crate::browser::normalize_possible_url;

const AX_SUCCESS: AXError = 0;
const AX_MESSAGING_TIMEOUT_SECS: f32 = 0.12;
const AX_TREE_BUDGET: Duration = Duration::from_millis(80);
const AX_TREE_NODE_LIMIT: usize = 160;

static AX_EVENT_PENDING: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone)]
pub(crate) struct NativeWindowSnapshot {
    pub window_title: Option<String>,
    pub browser_url: Option<String>,
    pub source: &'static str,
}

pub(crate) fn accessibility_is_trusted() -> bool {
    unsafe { AXIsProcessTrusted() }
}

pub(crate) fn take_accessibility_event() -> bool {
    AX_EVENT_PENDING.swap(false, Ordering::AcqRel)
}

pub(crate) fn read_window_snapshot(pid: i32, include_browser_url: bool) -> NativeWindowSnapshot {
    let app = AxElement::application(pid);
    let ax_window = app
        .as_ref()
        .and_then(|app| app.element_attribute("AXFocusedWindow"));
    let ax_title = ax_window
        .as_ref()
        .and_then(|window| window.string_attribute("AXTitle"))
        .and_then(clean_text);

    let browser_url = if include_browser_url {
        ax_window.as_ref().and_then(find_browser_url)
    } else {
        None
    };

    if ax_title.is_some() || browser_url.is_some() {
        return NativeWindowSnapshot {
            window_title: ax_title,
            browser_url,
            source: "nsworkspace-ax",
        };
    }

    NativeWindowSnapshot {
        window_title: core_graphics_window_title(pid),
        browser_url: None,
        source: "nsworkspace-coregraphics",
    }
}

pub(crate) struct AccessibilityEventMonitor {
    pid: Option<i32>,
    observer: Option<AxObserver>,
}

impl AccessibilityEventMonitor {
    pub(crate) fn new() -> Self {
        Self {
            pid: None,
            observer: None,
        }
    }

    pub(crate) fn bind(&mut self, pid: Option<i32>) {
        if self.pid == pid {
            if let Some(observer) = &mut self.observer {
                observer.refresh_window();
            }
            return;
        }

        self.observer = None;
        self.pid = pid;

        let Some(pid) = pid else {
            return;
        };
        match AxObserver::new(pid) {
            Ok(observer) => self.observer = Some(observer),
            Err(error) => {
                debug!(
                    pid,
                    ax_error = error,
                    "cannot attach accessibility observer"
                );
            }
        }
    }
}

struct AxElement(AXUIElementRef);

impl AxElement {
    fn application(pid: i32) -> Option<Self> {
        let raw = unsafe { AXUIElementCreateApplication(pid) };
        if raw.is_null() {
            return None;
        }
        let element = Self(raw);
        unsafe {
            let _ = AXUIElementSetMessagingTimeout(element.0, AX_MESSAGING_TIMEOUT_SECS);
        }
        Some(element)
    }

    fn element_attribute(&self, name: &str) -> Option<Self> {
        let value = self.copy_attribute(name)?;
        if unsafe { CFGetTypeID(value) } != unsafe { AXUIElementGetTypeID() } {
            unsafe { CFRelease(value) };
            return None;
        }
        Some(Self(value.cast_mut().cast()))
    }

    fn string_attribute(&self, name: &str) -> Option<String> {
        let value = self.copy_attribute(name)?;
        let value = unsafe { CFType::wrap_under_create_rule(value) };

        if let Some(string) = value.downcast::<CFString>() {
            return Some(string.to_string());
        }
        value
            .downcast::<CFURL>()
            .map(|url| url.get_string().to_string())
    }

    fn children(&self) -> Vec<Self> {
        let value = match self.copy_attribute("AXChildren") {
            Some(value) => value,
            None => return Vec::new(),
        };
        if unsafe { CFGetTypeID(value) } != unsafe { core_foundation::array::CFArrayGetTypeID() } {
            unsafe { CFRelease(value) };
            return Vec::new();
        }

        let array = value as CFArrayRef;
        let count = unsafe { CFArrayGetCount(array) };
        let mut children = Vec::with_capacity(usize::try_from(count).unwrap_or(0).min(32));
        for index in 0..count {
            let raw = unsafe { CFArrayGetValueAtIndex(array, index) } as CFTypeRef;
            if raw.is_null() || unsafe { CFGetTypeID(raw) } != unsafe { AXUIElementGetTypeID() } {
                continue;
            }
            unsafe { CFRetain(raw) };
            children.push(Self(raw.cast_mut().cast()));
        }
        unsafe { CFRelease(value) };
        children
    }

    fn copy_attribute(&self, name: &str) -> Option<CFTypeRef> {
        let name = CFString::new(name);
        let mut value: CFTypeRef = ptr::null();
        let error = unsafe {
            AXUIElementCopyAttributeValue(
                self.0,
                name.as_concrete_TypeRef(),
                &mut value as *mut CFTypeRef,
            )
        };
        if error == AX_SUCCESS && !value.is_null() {
            Some(value)
        } else {
            None
        }
    }
}

impl Drop for AxElement {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CFRelease(self.0.cast()) };
        }
    }
}

struct AxObserver {
    raw: AXObserverRef,
    app: AxElement,
    window: Option<AxElement>,
}

impl AxObserver {
    fn new(pid: i32) -> Result<Self, AXError> {
        let app = AxElement::application(pid).ok_or(kAXErrorCannotComplete)?;
        let mut raw: AXObserverRef = ptr::null_mut();
        let error = unsafe { AXObserverCreate(pid, accessibility_callback, &mut raw) };
        if error != AX_SUCCESS || raw.is_null() {
            return Err(error);
        }

        let mut observer = Self {
            raw,
            app,
            window: None,
        };
        for notification in [
            kAXFocusedWindowChangedNotification,
            kAXMainWindowChangedNotification,
            kAXFocusedUIElementChangedNotification,
            kAXWindowCreatedNotification,
        ] {
            observer.add_notification(notification, observer.app.0);
        }
        observer.refresh_window();

        unsafe {
            CFRunLoopAddSource(
                CFRunLoopGetCurrent(),
                AXObserverGetRunLoopSource(observer.raw),
                kCFRunLoopDefaultMode,
            );
        }
        Ok(observer)
    }

    fn refresh_window(&mut self) {
        let next = self.app.element_attribute("AXFocusedWindow");
        let unchanged = match (&self.window, &next) {
            (Some(current), Some(next)) => unsafe { CFEqual(current.0.cast(), next.0.cast()) != 0 },
            (None, None) => true,
            _ => false,
        };
        if unchanged {
            return;
        }

        if let Some(current) = &self.window {
            for notification in [
                kAXTitleChangedNotification,
                kAXValueChangedNotification,
                kAXSelectedChildrenChangedNotification,
            ] {
                self.remove_notification(notification, current.0);
            }
        }

        self.window = next;
        if let Some(window) = &self.window {
            let window_ref = window.0;
            for notification in [
                kAXTitleChangedNotification,
                kAXValueChangedNotification,
                kAXSelectedChildrenChangedNotification,
            ] {
                self.add_notification(notification, window_ref);
            }
        }
    }

    fn add_notification(&self, name: &str, element: AXUIElementRef) {
        let name = CFString::new(name);
        let error = unsafe {
            AXObserverAddNotification(
                self.raw,
                element,
                name.as_concrete_TypeRef(),
                ptr::null_mut(),
            )
        };
        if error != AX_SUCCESS
            && error != kAXErrorNotificationUnsupported
            && error != kAXErrorNotificationAlreadyRegistered
        {
            debug!(ax_error = error, notification = %name, "AX notification unavailable");
        }
    }

    fn remove_notification(&self, name: &str, element: AXUIElementRef) {
        let name = CFString::new(name);
        unsafe {
            let _ = AXObserverRemoveNotification(self.raw, element, name.as_concrete_TypeRef());
        }
    }
}

impl Drop for AxObserver {
    fn drop(&mut self) {
        if self.raw.is_null() {
            return;
        }
        unsafe {
            CFRunLoopRemoveSource(
                CFRunLoopGetCurrent(),
                AXObserverGetRunLoopSource(self.raw),
                kCFRunLoopDefaultMode,
            );
            CFRelease(self.raw.cast());
        }
    }
}

unsafe extern "C" fn accessibility_callback(
    _observer: AXObserverRef,
    _element: AXUIElementRef,
    _notification: CFStringRef,
    _context: *mut c_void,
) {
    AX_EVENT_PENDING.store(true, Ordering::Release);
}

fn find_browser_url(window: &AxElement) -> Option<String> {
    let started_at = Instant::now();
    let mut queue = VecDeque::from([(retained_element(window), 0usize)]);
    let mut visited = 0usize;
    let mut best: Option<(u8, String)> = None;

    while let Some((element, depth)) = queue.pop_front() {
        if visited >= AX_TREE_NODE_LIMIT || started_at.elapsed() >= AX_TREE_BUDGET {
            break;
        }
        visited += 1;

        for attribute in ["AXURL", "AXDocument"] {
            if let Some(url) = element
                .string_attribute(attribute)
                .as_deref()
                .and_then(normalize_possible_url)
            {
                return Some(url);
            }
        }

        let role = element.string_attribute("AXRole").unwrap_or_default();
        let is_text_input = matches!(role.as_str(), "AXTextField" | "AXComboBox");
        let is_document = matches!(role.as_str(), "AXWebArea" | "AXDocument");
        if is_text_input || is_document {
            let metadata = ["AXIdentifier", "AXDescription", "AXTitle", "AXHelp"]
                .into_iter()
                .filter_map(|name| element.string_attribute(name))
                .collect::<Vec<_>>()
                .join(" ")
                .to_ascii_lowercase();
            let address_like = [
                "address",
                "location",
                "omnibox",
                "url",
                "search bar",
                "地址",
                "网址",
            ]
            .iter()
            .any(|needle| metadata.contains(needle));

            if let Some(url) = element
                .string_attribute("AXValue")
                .as_deref()
                .and_then(normalize_possible_url)
            {
                let score = if address_like {
                    94
                } else if is_document {
                    84
                } else {
                    78
                };
                if best.as_ref().map(|(old, _)| score > *old).unwrap_or(true) {
                    best = Some((score, url));
                }
            }
        }

        if depth < 10 {
            queue.extend(
                element
                    .children()
                    .into_iter()
                    .map(|child| (child, depth + 1)),
            );
        }
    }

    if started_at.elapsed() >= AX_TREE_BUDGET {
        debug!(visited, "browser accessibility scan reached time budget");
    }
    best.map(|(_, url)| url)
}

fn retained_element(element: &AxElement) -> AxElement {
    unsafe { CFRetain(element.0.cast()) };
    AxElement(element.0)
}

fn core_graphics_window_title(pid: i32) -> Option<String> {
    let windows = unsafe {
        CGWindowListCopyWindowInfo(
            kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements,
            kCGNullWindowID,
        )
    };
    if windows.is_null() {
        return None;
    }

    let count = unsafe { CFArrayGetCount(windows) };
    let mut title = None;
    for index in 0..count {
        let dictionary = unsafe { CFArrayGetValueAtIndex(windows, index) } as CFDictionaryRef;
        if dictionary.is_null()
            || dictionary_i32(dictionary, unsafe { kCGWindowOwnerPID }) != Some(pid)
            || dictionary_i32(dictionary, unsafe { kCGWindowLayer }) != Some(0)
        {
            continue;
        }
        title = dictionary_string(dictionary, unsafe { kCGWindowName }).and_then(clean_text);
        if title.is_some() {
            break;
        }
    }
    unsafe { CFRelease(windows.cast()) };
    title
}

pub(crate) fn frontmost_window_id(pid: i32) -> Option<u32> {
    let windows = unsafe {
        CGWindowListCopyWindowInfo(
            kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements,
            kCGNullWindowID,
        )
    };
    if windows.is_null() {
        return None;
    }
    let count = unsafe { CFArrayGetCount(windows) };
    let mut result = None;
    for index in 0..count {
        let dictionary = unsafe { CFArrayGetValueAtIndex(windows, index) } as CFDictionaryRef;
        if dictionary.is_null()
            || dictionary_i32(dictionary, unsafe { kCGWindowOwnerPID }) != Some(pid)
            || dictionary_i32(dictionary, unsafe { kCGWindowLayer }) != Some(0)
        {
            continue;
        }
        result = dictionary_i32(dictionary, unsafe { kCGWindowNumber })
            .and_then(|value| u32::try_from(value).ok());
        if result.is_some() {
            break;
        }
    }
    unsafe { CFRelease(windows.cast()) };
    result
}

fn dictionary_value(dictionary: CFDictionaryRef, key: CFStringRef) -> Option<CFTypeRef> {
    let mut value: CFTypeRef = ptr::null();
    let found = unsafe {
        CFDictionaryGetValueIfPresent(dictionary, key.cast(), &mut value as *mut CFTypeRef)
    };
    (found != 0 && !value.is_null()).then_some(value)
}

fn dictionary_i32(dictionary: CFDictionaryRef, key: CFStringRef) -> Option<i32> {
    let value = dictionary_value(dictionary, key)?;
    let mut number = 0i32;
    let success = unsafe {
        CFNumberGetValue(
            value as CFNumberRef,
            kCFNumberSInt32Type,
            (&mut number as *mut i32).cast(),
        )
    };
    success.then_some(number)
}

fn dictionary_string(dictionary: CFDictionaryRef, key: CFStringRef) -> Option<String> {
    let value = dictionary_value(dictionary, key)?;
    if unsafe { CFGetTypeID(value) } != CFString::type_id() {
        return None;
    }
    Some(unsafe { CFString::wrap_under_get_rule(value as CFStringRef) }.to_string())
}

fn clean_text(value: String) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

pub(crate) fn log_accessibility_status() {
    if accessibility_is_trusted() {
        debug!("macOS Accessibility permission available");
    } else {
        warn!(
            "macOS Accessibility permission is not granted; using CoreGraphics and low-frequency fallback, so some in-app tab changes and browser URLs may be delayed or unavailable"
        );
    }
}
