//! The tray icon: what the app is, and the handful of things a person does
//! without opening a window.
//!
//! One hidden window owns the icon and its menu, on its own message loop; a
//! click becomes an [`Action`] on a channel the app drains. Nothing here
//! reaches into the app — the app decides what an action means.
//!
//! [`open_url`] and [`open_folder`] are here rather than in the app because the
//! shell call is the same Win32 call the menu already uses.

use std::sync::mpsc::Receiver;

/// What a menu item asks the app to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    AutoStart,
    Pair,
    Editor,
    Folder,
    Exit,
}

#[derive(Clone)]
pub struct Status {
    pub tooltip: String,
    pub autostart: bool,
}

/// A tray whose icon could not be created: the app still runs, with one surface
/// missing, rather than refusing to start.
pub struct Tray {
    actions: Receiver<Action>,
}

impl Tray {
    /// Every action a person asked for since the last drain.
    pub fn actions(&self) -> &Receiver<Action> {
        &self.actions
    }
}

#[cfg(windows)]
mod platform {
    use super::{Action, Status, Tray};
    use std::path::Path;
    use std::sync::mpsc::{self, Sender};
    use std::sync::{Mutex, OnceLock};

    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::Shell::{
        NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW,
        Shell_NotifyIconW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
        GWLP_USERDATA, GetCursorPos, GetMessageW, GetWindowLongPtrW, IDI_APPLICATION, LoadIconW,
        MF_CHECKED, MF_GRAYED, MF_SEPARATOR, MF_STRING, MSG, PostMessageW, PostQuitMessage,
        RegisterClassW, SetForegroundWindow, SetWindowLongPtrW, TPM_RETURNCMD, TPM_RIGHTBUTTON,
        TrackPopupMenu, WM_APP, WM_CLOSE, WM_DESTROY, WM_LBUTTONUP, WM_NULL, WM_RBUTTONUP,
        WNDCLASSW, WS_POPUP,
    };

    const CALLBACK: u32 = WM_APP + 1;
    const UPDATE: u32 = WM_APP + 2;
    /// A tooltip longer than this is cut by the shell anyway.
    const TOOLTIP_CHARS: usize = 63;

    /// The actions a click produced, and what the menu shows.
    struct Context {
        actions: Sender<Action>,
        status: Mutex<Status>,
    }

    /// The window the app talks to. A raw handle is not `Send`, so the app holds
    /// the numeric value and the platform converts it back on its own thread.
    static WINDOW: OnceLock<isize> = OnceLock::new();

    pub fn spawn(status: Status) -> Option<Tray> {
        let (actions, received) = mpsc::channel();
        let (ready, started) = mpsc::channel::<Option<isize>>();
        std::thread::spawn(move || run(status, actions, ready));
        match started.recv() {
            Ok(Some(window)) => {
                let _ = WINDOW.set(window);
                Some(Tray { actions: received })
            }
            _ => None,
        }
    }

    /// Shows a newer tooltip and check state. A tray that never started is a
    /// no-op, so a failed icon is not an error the app has to keep.
    pub fn update(status: Status) {
        let Some(window) = WINDOW.get().copied() else {
            return;
        };
        let boxed = Box::into_raw(Box::new(status));
        // SAFETY: the pointer is a live `Box<Status>` and the window's message
        // loop takes ownership of it in `wndproc`.
        unsafe {
            PostMessageW(window as HWND, UPDATE, 0, boxed as LPARAM);
        }
    }

    fn run(status: Status, actions: Sender<Action>, ready: Sender<Option<isize>>) {
        let class = wide("YunariTrayWindow");
        let window_name = wide("Yunari");
        let context = Box::into_raw(Box::new(Context {
            actions,
            status: Mutex::new(status),
        }));
        // SAFETY: the class name and window name outlive the call, and the
        // context pointer stays alive until the message loop ends below.
        unsafe {
            let instance = GetModuleHandleW(std::ptr::null());
            let class = WNDCLASSW {
                style: 0,
                lpfnWndProc: Some(wndproc),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: instance,
                hIcon: LoadIconW(std::ptr::null_mut(), IDI_APPLICATION),
                hCursor: std::ptr::null_mut(),
                hbrBackground: std::ptr::null_mut(),
                lpszMenuName: std::ptr::null(),
                lpszClassName: class.as_ptr(),
            };
            RegisterClassW(&class);
            let window = CreateWindowExW(
                0,
                class.lpszClassName,
                window_name.as_ptr(),
                WS_POPUP,
                0,
                0,
                0,
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                instance,
                std::ptr::null(),
            );
            if window.is_null() {
                drop(Box::from_raw(context));
                let _ = ready.send(None);
                return;
            }
            SetWindowLongPtrW(window, GWLP_USERDATA, context as isize);
            let mut icon = NOTIFYICONDATAW {
                cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
                hWnd: window,
                uID: 1,
                uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
                uCallbackMessage: CALLBACK,
                hIcon: LoadIconW(std::ptr::null_mut(), IDI_APPLICATION),
                ..Default::default()
            };
            if !add_icon(NIM_ADD, &mut icon, &context_status(context)) {
                DestroyWindow(window);
                drop(Box::from_raw(context));
                let _ = ready.send(None);
                return;
            }
            let _ = ready.send(Some(window as isize));
            let mut message = std::mem::zeroed::<MSG>();
            while GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) > 0 {
                windows_sys::Win32::UI::WindowsAndMessaging::TranslateMessage(&message);
                windows_sys::Win32::UI::WindowsAndMessaging::DispatchMessageW(&message);
            }
        }
        drop(unsafe { Box::from_raw(context) });
    }

    fn context_status(context: *mut Context) -> Status {
        // SAFETY: the pointer is the live context this thread owns.
        unsafe { (*context).status.lock().expect("the tray status").clone() }
    }

    /// Adds, changes or removes the icon; `false` means the shell refused it.
    unsafe fn add_icon(message: u32, icon: &mut NOTIFYICONDATAW, status: &Status) -> bool {
        icon.szTip = [0; 128];
        for (slot, unit) in icon
            .szTip
            .iter_mut()
            .zip(status.tooltip.encode_utf16().take(TOOLTIP_CHARS))
        {
            *slot = unit;
        }
        // SAFETY: `icon` is a filled `NOTIFYICONDATAW` that outlives the call.
        unsafe { Shell_NotifyIconW(message, icon) != 0 }
    }

    unsafe extern "system" fn wndproc(
        window: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        // SAFETY: the window data is the context pointer `run` installed and
        // keeps alive for as long as the window exists.
        let context = unsafe { GetWindowLongPtrW(window, GWLP_USERDATA) } as *mut Context;
        match message {
            CALLBACK => {
                let event = (lparam as u32) & 0xffff;
                if (event == WM_RBUTTONUP || event == WM_LBUTTONUP) && !context.is_null() {
                    // SAFETY: the pointer was checked above.
                    unsafe { show_menu(window, context) };
                }
                0
            }
            UPDATE => {
                if !context.is_null() {
                    // SAFETY: `update` posted an owned `Box<Status>`.
                    let status = *unsafe { Box::from_raw(lparam as *mut Status) };
                    // SAFETY: the pointer was checked above.
                    unsafe {
                        (*context)
                            .status
                            .lock()
                            .expect("the tray status")
                            .clone_from(&status);
                        let mut icon = NOTIFYICONDATAW {
                            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
                            hWnd: window,
                            uID: 1,
                            uFlags: NIF_TIP,
                            ..Default::default()
                        };
                        add_icon(NIM_MODIFY, &mut icon, &status);
                    }
                }
                0
            }
            WM_CLOSE => {
                unsafe { DestroyWindow(window) };
                0
            }
            WM_DESTROY => {
                let icon = NOTIFYICONDATAW {
                    cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
                    hWnd: window,
                    uID: 1,
                    ..Default::default()
                };
                // SAFETY: removing an icon that was added is always allowed.
                unsafe {
                    Shell_NotifyIconW(NIM_DELETE, &icon);
                    PostQuitMessage(0);
                }
                0
            }
            _ => unsafe { DefWindowProcW(window, message, wparam, lparam) },
        }
    }

    unsafe fn show_menu(window: HWND, context: *mut Context) {
        // SAFETY: `context` is the live context owned by this thread.
        let status = unsafe { (*context).status.lock().expect("the tray status").clone() };
        let menu = unsafe { CreatePopupMenu() };
        if menu.is_null() {
            return;
        }
        let status_line = wide(&status.tooltip);
        let mut point = POINT { x: 0, y: 0 };
        unsafe {
            AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, status_line.as_ptr());
            AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
            let autostart = wide("Auto start");
            let flags = if status.autostart {
                MF_STRING | MF_CHECKED
            } else {
                MF_STRING
            };
            AppendMenuW(menu, flags, ITEM_AUTOSTART, autostart.as_ptr());
            for (id, label) in [
                (ITEM_PAIR, "Pair display…"),
                (ITEM_EDITOR, "Theme Editor"),
                (ITEM_FOLDER, "Open Yunari folder"),
                (ITEM_EXIT, "Exit"),
            ] {
                let text = wide(label);
                // Every action is live; a row the app cannot complete says so
                // in the window it opens rather than here.
                AppendMenuW(menu, MF_STRING, id, text.as_ptr());
            }
            GetCursorPos(&mut point);
            SetForegroundWindow(window);
            let command = TrackPopupMenu(
                menu,
                TPM_RETURNCMD | TPM_RIGHTBUTTON,
                point.x,
                point.y,
                0,
                window,
                std::ptr::null(),
            ) as usize;
            PostMessageW(window, WM_NULL, 0, 0);
            DestroyMenu(menu);
            let action = match command {
                ITEM_AUTOSTART => Some(Action::AutoStart),
                ITEM_PAIR => Some(Action::Pair),
                ITEM_EDITOR => Some(Action::Editor),
                ITEM_FOLDER => Some(Action::Folder),
                ITEM_EXIT => Some(Action::Exit),
                _ => None,
            };
            if let Some(action) = action {
                let _ = (*context).actions.send(action);
            }
        }
    }

    const ITEM_AUTOSTART: usize = 1;
    const ITEM_PAIR: usize = 2;
    const ITEM_EDITOR: usize = 3;
    const ITEM_FOLDER: usize = 4;
    const ITEM_EXIT: usize = 5;

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub fn open_url(url: &str) -> bool {
        shell_open(url)
    }

    pub fn open_folder(path: &Path) -> bool {
        shell_open(&path.display().to_string())
    }

    fn shell_open(target: &str) -> bool {
        use windows_sys::Win32::UI::Shell::ShellExecuteW;
        use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        let operation = wide("open");
        let file = wide(target);
        // SAFETY: both strings outlive the call, and the return value is only
        // compared with the error boundary.
        let result = unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                operation.as_ptr(),
                file.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                SW_SHOWNORMAL,
            )
        };
        result as isize > 32
    }
}

#[cfg(windows)]
pub use platform::{open_folder, open_url, spawn, update};

#[cfg(not(windows))]
pub fn spawn(_status: Status) -> Option<Tray> {
    None
}

#[cfg(not(windows))]
pub fn update(_status: Status) {}

#[cfg(not(windows))]
pub fn open_url(_url: &str) -> bool {
    false
}

#[cfg(not(windows))]
pub fn open_folder(_path: &std::path::Path) -> bool {
    false
}
