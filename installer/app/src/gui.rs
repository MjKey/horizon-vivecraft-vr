//! The installer window (plain Win32): three folder rows, a log and the four buttons. Each action
//! runs on a worker thread that posts its log lines and result back to the window.
use crate::R;
use std::ptr::null_mut;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{COLOR_BTNFACE, DEFAULT_GUI_FONT, GetStockObject, HBRUSH};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const ID_UNINSTALL: usize = 10;
const ID_UPDATE: usize = 11;
const ID_REPAIR: usize = 12;
const ID_INSTALL: usize = 13;
const ID_BROWSE: usize = 20; // + row
const WM_LOG: u32 = WM_APP + 1;
const WM_DONE: u32 = WM_APP + 2;
const WM_STATUS: u32 = WM_APP + 3;
const EM_SETSEL: u32 = 0x00B1;
const EM_REPLACESEL: u32 = 0x00C2;

struct Ui {
    status: HWND,
    boxes: [HWND; 3],
    log: HWND,
    buttons: [HWND; 4],
}

static mut UI: Option<Ui> = None;

fn ui() -> &'static Ui {
    // SAFETY: set once in `run` before the message loop; only the window thread reads it.
    unsafe { (*std::ptr::addr_of!(UI)).as_ref().expect("window made") }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn text_of(h: HWND) -> String {
    // SAFETY: the buffer is as long as the window says its text is.
    unsafe {
        let n = GetWindowTextLengthW(h);
        let mut buf = vec![0u16; n as usize + 1];
        let got = GetWindowTextW(h, buf.as_mut_ptr(), buf.len() as i32);
        String::from_utf16_lossy(&buf[..got as usize])
    }
}

fn set_text(h: HWND, s: &str) {
    // SAFETY: a nul-terminated string that outlives the call.
    unsafe { SetWindowTextW(h, wide(s).as_ptr()) };
}

/// Posts `value` to the window; the window thread takes the box back.
fn post<T>(hwnd: isize, msg: u32, value: T) {
    let p = Box::into_raw(Box::new(value));
    // SAFETY: the window procedure turns `p` back into its box for each of these messages; when
    // the window is gone already, nothing will, so it is freed here.
    unsafe {
        if PostMessageW(hwnd as HWND, msg, 0, p as LPARAM) == 0 {
            drop(Box::from_raw(p));
        }
    }
}

fn child(parent: HWND, class: &str, text: &str, style: u32, ex: u32, x: i32, y: i32, w: i32, h: i32, id: usize) -> HWND {
    // SAFETY: plain window creation; the strings outlive the call.
    unsafe {
        let hwnd = CreateWindowExW(
            ex,
            wide(class).as_ptr(),
            wide(text).as_ptr(),
            WS_CHILD | WS_VISIBLE | style,
            x,
            y,
            w,
            h,
            parent,
            id as _,
            GetModuleHandleW(null_mut()),
            null_mut(),
        );
        SendMessageW(hwnd, WM_SETFONT, GetStockObject(DEFAULT_GUI_FONT) as WPARAM, 1);
        hwnd
    }
}

fn enable_buttons(on: bool) {
    for b in ui().buttons {
        // SAFETY: a window of ours.
        unsafe { EnableWindow(b, on as i32) };
    }
}

/// The installed and latest versions, on the status line; the Update button only when newer.
fn refresh_status(hwnd: HWND) {
    let dest = crate::full_path(&text_of(ui().boxes[0]));
    let h = hwnd as isize;
    std::thread::spawn(move || {
        let have = crate::read_version(&dest.join("version.txt"));
        let have_text = have.as_ref().map_or("Not installed".to_owned(), |v| format!("Installed: {v}"));
        let (text, update) = match crate::get_latest() {
            Ok(l) if have.as_ref().is_some_and(|h| l.version > *h) => {
                (format!("{have_text}. Update available: {}", l.version), true)
            }
            Ok(_) if have.is_some() => (format!("{have_text} (up to date)"), false),
            Ok(l) => (format!("{have_text}. Latest: {}", l.version), false),
            Err(_) => (format!("{have_text}. Couldn't check for updates."), have.is_some()),
        };
        post(h, WM_STATUS, (text, update));
    });
}

type Action = fn(&str, &str, &str, crate::Say) -> R<()>;

/// Runs one action with the buttons off, then shows its result or error.
fn act(hwnd: HWND, action: Action, done: &'static str, what: &'static str) {
    let [a, b, c] = ui().boxes.map(text_of);
    enable_buttons(false);
    let h = hwnd as isize;
    std::thread::spawn(move || {
        let say = |t: &str| post(h, WM_LOG, t.to_owned());
        let r = action(&a, &b, &c, &say);
        let msg = match r {
            Ok(()) => (true, done.to_owned()),
            Err(e) => {
                say(&format!("Error: {e}"));
                (false, format!("{what} failed: {e}"))
            }
        };
        post(h, WM_DONE, msg);
    });
}

extern "system" fn proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    // SAFETY: each WM_APP message's lparam is the box `post` made for that message's type.
    unsafe {
        match msg {
            WM_COMMAND => {
                let id = w & 0xFFFF;
                match id {
                    ID_INSTALL => act(
                        hwnd,
                        |a, b, c, s| crate::install(a, b, c, s, None),
                        "Horizon VR is installed. Start SteamVR, then open \"Horizon VR\" on your desktop.",
                        "Install",
                    ),
                    ID_REPAIR => act(
                        hwnd,
                        crate::repair,
                        "Horizon VR is repaired. Start SteamVR, then open \"Horizon VR\" on your desktop.",
                        "Repair",
                    ),
                    ID_UPDATE => act(
                        hwnd,
                        |a, b, c, s| {
                            if crate::update(a, b, c, s)? { Ok(()) } else { Err("Horizon VR is already up to date.".into()) }
                        },
                        "Horizon VR is updated. Start SteamVR, then open \"Horizon VR\" on your desktop.",
                        "Update",
                    ),
                    ID_UNINSTALL => {
                        let dest = text_of(ui().boxes[0]);
                        let q = format!("Uninstall Horizon VR from {dest}? Your worlds and settings are kept.");
                        let r = MessageBoxW(hwnd, wide(&q).as_ptr(), wide("Uninstall Horizon VR").as_ptr(), MB_YESNO | MB_ICONQUESTION);
                        if r == IDYES {
                            act(hwnd, |a, _, _, s| crate::uninstall(a, s), "Horizon VR is uninstalled.", "Uninstall");
                        }
                    }
                    _ if (ID_BROWSE..ID_BROWSE + 3).contains(&id) => {
                        let row = id - ID_BROWSE;
                        let tb = ui().boxes[row];
                        let cur = text_of(tb);
                        let picked = if row == 2 {
                            rfd::FileDialog::new().add_filter("Server jar", &["jar"]).pick_file()
                        } else {
                            rfd::FileDialog::new().set_directory(&cur).pick_folder()
                        };
                        if let Some(p) = picked {
                            set_text(tb, &p.display().to_string());
                        }
                    }
                    _ => {}
                }
                0
            }
            WM_LOG => {
                let t = *Box::from_raw(l as *mut String);
                let log = ui().log;
                let n = GetWindowTextLengthW(log) as usize;
                SendMessageW(log, EM_SETSEL, n, n as LPARAM);
                SendMessageW(log, EM_REPLACESEL, 0, wide(&format!("{t}\r\n")).as_ptr() as LPARAM);
                0
            }
            WM_DONE => {
                let (_, text) = *Box::from_raw(l as *mut (bool, String));
                MessageBoxW(hwnd, wide(&text).as_ptr(), wide("Horizon VR Installer").as_ptr(), MB_OK);
                enable_buttons(true);
                refresh_status(hwnd);
                0
            }
            WM_STATUS => {
                let (text, update) = *Box::from_raw(l as *mut (String, bool));
                set_text(ui().status, &text);
                EnableWindow(ui().buttons[1], update as i32);
                0
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                0
            }
            _ => DefWindowProcW(hwnd, msg, w, l),
        }
    }
}

pub fn run(install_dir: String) {
    // SAFETY: standard Win32 window setup and message loop on this thread.
    unsafe {
        let inst = GetModuleHandleW(null_mut());
        let class = wide("HorizonVRInstaller");
        let wc = WNDCLASSW {
            style: 0,
            lpfnWndProc: Some(proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: inst,
            hIcon: LoadIconW(inst, 1 as _),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hbrBackground: (COLOR_BTNFACE + 1) as usize as HBRUSH,
            lpszMenuName: null_mut(),
            lpszClassName: class.as_ptr(),
        };
        RegisterClassW(&wc);
        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
        let mut r = windows_sys::Win32::Foundation::RECT { left: 0, top: 0, right: 560, bottom: 430 };
        AdjustWindowRect(&mut r, style, 0);
        let (w, h) = (r.right - r.left, r.bottom - r.top);
        let x = (GetSystemMetrics(SM_CXSCREEN) - w) / 2;
        let y = (GetSystemMetrics(SM_CYSCREEN) - h) / 2;
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            wide("Horizon VR Installer").as_ptr(),
            style,
            x,
            y,
            w,
            h,
            null_mut(),
            null_mut(),
            inst,
            null_mut(),
        );
        let status = child(hwnd, "STATIC", "", 0, 0, 12, 10, 536, 20, 0);
        let rows = [
            ("Install folder", install_dir),
            ("Minecraft folder with 26.3 (found automatically)", crate::find_minecraft().display().to_string()),
            (
                "Minecraft 26.3 server jar (found automatically, or download it from minecraft.net)",
                crate::find_server_jar(),
            ),
        ];
        let mut boxes = [null_mut(); 3];
        let mut y = 38;
        for (i, (label, value)) in rows.iter().enumerate() {
            child(hwnd, "STATIC", label, 0, 0, 12, y, 536, 18, 0);
            boxes[i] = child(hwnd, "EDIT", value, WS_TABSTOP | ES_AUTOHSCROLL as u32, WS_EX_CLIENTEDGE, 12, y + 20, 440, 23, 0);
            child(hwnd, "BUTTON", "Browse...", WS_TABSTOP, 0, 460, y + 19, 88, 25, ID_BROWSE + i);
            y += 56;
        }
        let log = child(
            hwnd,
            "EDIT",
            "",
            WS_VSCROLL | ES_MULTILINE as u32 | ES_READONLY as u32 | ES_AUTOVSCROLL as u32,
            WS_EX_CLIENTEDGE,
            12,
            y,
            536,
            150,
            0,
        );
        let button = |text, x, id, def: bool| {
            let s = WS_TABSTOP | if def { BS_DEFPUSHBUTTON as u32 } else { 0 };
            child(hwnd, "BUTTON", text, s, 0, x, y + 160, 100, 30, id)
        };
        let buttons = [
            button("Uninstall", 12, ID_UNINSTALL, false),
            button("Update", 232, ID_UPDATE, false),
            button("Repair", 340, ID_REPAIR, false),
            button("Install", 448, ID_INSTALL, true),
        ];
        *std::ptr::addr_of_mut!(UI) = Some(Ui { status, boxes, log, buttons });
        ShowWindow(hwnd, SW_SHOW);
        refresh_status(hwnd);
        let mut m = std::mem::zeroed::<MSG>();
        while GetMessageW(&mut m, null_mut(), 0, 0) > 0 {
            if IsDialogMessageW(hwnd, &m) == 0 {
                TranslateMessage(&m);
                DispatchMessageW(&m);
            }
        }
    }
}
