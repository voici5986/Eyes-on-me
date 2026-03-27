pub const DEFAULT_IDLE_TIMEOUT_SECS: u64 = 180;

pub fn is_idle(timeout_secs: u64) -> bool {
    get_idle_seconds() >= timeout_secs
}

#[cfg(target_os = "windows")]
fn get_idle_seconds() -> u64 {
    use std::mem::size_of;
    use winapi::um::sysinfoapi::GetTickCount;
    use winapi::um::winuser::{GetLastInputInfo, LASTINPUTINFO};

    unsafe {
        let mut lii = LASTINPUTINFO {
            cbSize: size_of::<LASTINPUTINFO>() as u32,
            dwTime: 0,
        };

        if GetLastInputInfo(&mut lii) == 0 {
            return 0;
        }

        let current_tick = GetTickCount();
        let idle_ms = if current_tick >= lii.dwTime {
            current_tick - lii.dwTime
        } else {
            (u32::MAX - lii.dwTime) + current_tick + 1
        };

        (idle_ms / 1000) as u64
    }
}

#[cfg(target_os = "macos")]
fn get_idle_seconds() -> u64 {
    use core_graphics::event_source::CGEventSourceStateID;

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGEventSourceSecondsSinceLastEventType(
            state_id: CGEventSourceStateID,
            event_type: u32,
        ) -> f64;
    }

    const K_CG_ANY_INPUT_EVENT_TYPE: u32 = !0u32;

    let idle_time = unsafe {
        CGEventSourceSecondsSinceLastEventType(
            CGEventSourceStateID::HIDSystemState,
            K_CG_ANY_INPUT_EVENT_TYPE,
        )
    };

    if idle_time.is_sign_negative() {
        0
    } else {
        idle_time as u64
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn get_idle_seconds() -> u64 {
    0
}
