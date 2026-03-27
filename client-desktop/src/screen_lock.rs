#[cfg(target_os = "macos")]
pub fn is_locked() -> bool {
    use core_foundation::base::{CFRelease, CFTypeRef, TCFType};
    use core_foundation::boolean::CFBoolean;
    use core_foundation::dictionary::CFDictionaryRef;
    use core_foundation::string::CFString;
    use std::process::Command;

    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn CGSessionCopyCurrentDictionary() -> CFDictionaryRef;
    }

    unsafe {
        let dict = CGSessionCopyCurrentDictionary();
        if dict.is_null() {
            return true;
        }

        let key = CFString::new("CGSSessionScreenIsLocked");
        let mut value_ref: CFTypeRef = std::ptr::null();
        let found = core_foundation::dictionary::CFDictionaryGetValueIfPresent(
            dict,
            key.as_CFTypeRef() as *const _,
            &mut value_ref,
        );

        let session_locked = if found != 0 && !value_ref.is_null() {
            let cf_bool = CFBoolean::wrap_under_get_rule(value_ref as _);
            cf_bool == CFBoolean::true_value()
        } else {
            false
        };

        CFRelease(dict as _);
        if session_locked {
            return true;
        }
    }

    if let Ok(out) = Command::new("pgrep")
        .args(["-x", "ScreenSaverEngine"])
        .output()
    {
        if out.status.success() {
            return true;
        }
    }

    if let Ok(out) = Command::new("ioreg")
        .args(["-r", "-c", "IODisplayWrangler", "-d", "1"])
        .output()
    {
        let stdout = String::from_utf8_lossy(&out.stdout);
        if stdout.contains("\"DevicePowerState\" = 0") {
            return true;
        }
    }

    false
}

#[cfg(target_os = "windows")]
pub fn is_locked() -> bool {
    use winapi::um::winnt::GENERIC_ALL;
    use winapi::um::winuser::{CloseDesktop, OpenInputDesktop, SwitchDesktop};

    unsafe {
        let desktop = OpenInputDesktop(0, 0, GENERIC_ALL);
        if desktop.is_null() {
            return true;
        }

        let switched = SwitchDesktop(desktop);
        CloseDesktop(desktop);
        switched == 0
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn is_locked() -> bool {
    false
}
