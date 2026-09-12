//! Windows single-instance ownership and activation, independent of HTTP sharing.
use std::{
    ffi::OsStr,
    io,
    os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    ptr,
};
use windows_sys::Win32::{
    Foundation::{GetLastError, ERROR_ALREADY_EXISTS, HWND, LPARAM, WAIT_OBJECT_0},
    System::Threading::{CreateEventW, CreateMutexW, SetEvent, WaitForSingleObject, INFINITE},
    UI::WindowsAndMessaging::{
        AllowSetForegroundWindow, EnumWindows, GetPropW, GetWindowThreadProcessId, SetPropW,
    },
};

const INSTANCE_NAME: &str = "Local\\FileShare.SingleInstance";
const WINDOW_PROPERTY: &str = "com.fileshare.desktop.MainWindow";

pub struct InstanceGuard {
    _mutex: OwnedHandle,
    activation: OwnedHandle,
}

fn wide(value: &str) -> Vec<u16> {
    OsStr::new(value).encode_wide().chain(Some(0)).collect()
}

pub fn acquire_or_notify() -> io::Result<Option<InstanceGuard>> {
    acquire_named(INSTANCE_NAME)
}

fn acquire_named(name: &str) -> io::Result<Option<InstanceGuard>> {
    // Create the auto-reset event before claiming the mutex, so a second launch
    // can signal even while the first process is still initializing its window.
    let event_name = wide(&format!("{name}.Activate"));
    let event = unsafe { CreateEventW(ptr::null(), 0, 0, event_name.as_ptr()) };
    if event.is_null() {
        return Err(io::Error::last_os_error());
    }
    // Every handle returned by Create* is owned here and closed on all paths.
    let activation = unsafe { OwnedHandle::from_raw_handle(event) };
    let mutex_name = wide(name);
    let mutex = unsafe { CreateMutexW(ptr::null(), 0, mutex_name.as_ptr()) };
    let already_running = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
    if mutex.is_null() {
        return Err(io::Error::last_os_error());
    }
    let mutex = unsafe { OwnedHandle::from_raw_handle(mutex) };
    if already_running {
        // Delegate foreground permission only to our marked main window's PID.
        // A hidden/minimized window still appears in EnumWindows.
        if name == INSTANCE_NAME {
            unsafe { EnumWindows(Some(allow_existing_foreground), 0) };
        }
        if unsafe { SetEvent(activation.as_raw_handle()) } == 0 {
            return Err(io::Error::last_os_error());
        }
        return Ok(None);
    }
    Ok(Some(InstanceGuard {
        _mutex: mutex,
        activation,
    }))
}

pub fn mark_main_window(hwnd: HWND) -> io::Result<()> {
    let property = wide(WINDOW_PROPERTY);
    // The property is just a marker; it owns no extra resource.
    if unsafe { SetPropW(hwnd, property.as_ptr(), hwnd) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

unsafe extern "system" fn allow_existing_foreground(hwnd: HWND, _: LPARAM) -> i32 {
    let property = wide(WINDOW_PROPERTY);
    if !GetPropW(hwnd, property.as_ptr()).is_null() {
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid != 0 {
            AllowSetForegroundWindow(pid);
        }
        return 0;
    }
    1
}

impl InstanceGuard {
    pub fn listen(&self, on_activate: impl Fn() + Send + 'static) -> io::Result<()> {
        let activation = self.activation.try_clone()?;
        std::thread::Builder::new()
            .name("fileshare-activation".into())
            .spawn(move || loop {
                let result = unsafe { WaitForSingleObject(activation.as_raw_handle(), INFINITE) };
                if result != WAIT_OBJECT_0 {
                    eprintln!(
                        "FileShare activation listener failed: {}",
                        io::Error::last_os_error()
                    );
                    break;
                }
                on_activate();
            })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn secondary_process() {
        let Ok(name) = std::env::var("FILESHARE_TEST_INSTANCE_NAME") else {
            return;
        };
        assert!(acquire_named(&name).unwrap().is_none());
    }

    #[test]
    fn activation_received_before_listener_starts_is_delivered() {
        let name = format!("Local\\FileShare.Test.{}", uuid::Uuid::new_v4());
        let primary = acquire_named(&name).unwrap().unwrap();
        assert!(acquire_named(&name).unwrap().is_none());
        let (send, receive) = std::sync::mpsc::channel();
        primary
            .listen(move || {
                let _ = send.send(());
            })
            .unwrap();
        receive
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        assert!(acquire_named(&name).unwrap().is_none());
        receive
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
    }

    #[test]
    fn repeated_launch_notifies_same_instance_and_lock_is_reusable() {
        let name = format!("Local\\FileShare.Test.{}", uuid::Uuid::new_v4());
        let primary = acquire_named(&name).unwrap().unwrap();
        for _ in 0..3 {
            let status = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "single_instance::tests::secondary_process"])
                .env("FILESHARE_TEST_INSTANCE_NAME", &name)
                .status()
                .unwrap();
            assert!(status.success());
            // No listener was waiting yet: activation must survive startup.
            assert_eq!(
                unsafe { WaitForSingleObject(primary.activation.as_raw_handle(), 1000) },
                WAIT_OBJECT_0
            );
        }
        drop(primary);
        assert!(acquire_named(&name).unwrap().is_some());
    }
}
