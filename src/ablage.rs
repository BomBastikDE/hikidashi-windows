//! Zugriff auf die Windows-Zwischenablage, Einfügen per Strg+V und Autostart.

use crate::formate::{self, Rgba};
use crate::verlauf::{MAX_BILD_BYTES, MAX_TEXT_BYTES};
use std::ffi::c_void;
use std::time::Duration;
use windows::core::{w, PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, GlobalFree, ERROR_SUCCESS, HANDLE, HGLOBAL, HWND};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, GetClipboardOwner,
    IsClipboardFormatAvailable, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::Memory::{
    GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE,
};
use windows::Win32::System::Ole::{CF_DIB, CF_HDROP, CF_UNICODETEXT};
use windows::Win32::System::Registry::{
    RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ,
};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
    VIRTUAL_KEY, VK_CONTROL, VK_V,
};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

pub enum Inhalt {
    Text(String),
    Dateien(Vec<String>),
    Bild(Rgba),
}

/// Schließt die Zwischenablage beim Verlassen des Gültigkeitsbereichs.
pub(crate) struct Offen;

impl Drop for Offen {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseClipboard();
        }
    }
}

/// Andere Programme halten die Zwischenablage manchmal kurz offen: einige Male versuchen.
pub(crate) fn oeffnen(hwnd: HWND) -> Option<Offen> {
    for _ in 0..10 {
        if unsafe { OpenClipboard(Some(hwnd)) }.is_ok() {
            return Some(Offen);
        }
        std::thread::sleep(Duration::from_millis(15));
    }
    None
}

pub(crate) fn format(name: PCWSTR) -> u32 {
    unsafe { RegisterClipboardFormatW(name) }
}

fn vorhanden(fmt: u32) -> bool {
    unsafe { IsClipboardFormatAvailable(fmt) }.is_ok()
}

/// Kopiert die Daten eines Formats, wenn sie nicht größer als `max` sind.
fn daten(fmt: u32, max: usize) -> Option<Vec<u8>> {
    unsafe {
        let h = GetClipboardData(fmt).ok()?;
        let g = HGLOBAL(h.0);
        let groesse = GlobalSize(g);
        if groesse == 0 || groesse > max {
            return None;
        }
        let p = GlobalLock(g) as *const u8;
        if p.is_null() {
            return None;
        }
        let v = std::slice::from_raw_parts(p, groesse).to_vec();
        let _ = GlobalUnlock(g);
        Some(v)
    }
}

/// Inhalte, die ihr Absender als geheim oder „nicht in den Verlauf“ markiert hat.
/// Muss bei geöffneter Zwischenablage aufgerufen werden.
fn ausgeschlossen() -> bool {
    if vorhanden(format(w!("ExcludeClipboardContentFromMonitorProcessing")))
        || vorhanden(format(w!("Clipboard Viewer Ignore")))
    {
        return true;
    }
    // Windows-eigene Markierung „nicht in den Zwischenablageverlauf“ (DWORD 0).
    let f = format(w!("CanIncludeInClipboardHistory"));
    if vorhanden(f) {
        if let Some(d) = daten(f, 64) {
            if d.len() >= 4 && u32::from_le_bytes([d[0], d[1], d[2], d[3]]) == 0 {
                return true;
            }
        }
    }
    false
}

/// Liest den aktuellen Inhalt: Dateien vor Text vor Bild.
pub fn lesen(hwnd: HWND) -> Option<Inhalt> {
    let _offen = oeffnen(hwnd)?;
    if ausgeschlossen() {
        return None;
    }
    let hdrop = CF_HDROP.0 as u32;
    if vorhanden(hdrop) {
        if let Some(d) = daten(hdrop, 4 << 20) {
            let pfade = formate::dropfiles_lesen(&d);
            if !pfade.is_empty() {
                return Some(Inhalt::Dateien(pfade));
            }
        }
    }
    let text = CF_UNICODETEXT.0 as u32;
    if vorhanden(text) {
        let d = daten(text, MAX_TEXT_BYTES * 2 + 2)?;
        let einheiten: Vec<u16> = d
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .take_while(|&c| c != 0)
            .collect();
        return Some(Inhalt::Text(String::from_utf16_lossy(&einheiten)));
    }
    let dib = CF_DIB.0 as u32;
    if vorhanden(dib) {
        let d = daten(dib, MAX_BILD_BYTES + 4096)?;
        return formate::dib_zu_rgba(&d, MAX_BILD_BYTES).map(Inhalt::Bild);
    }
    None
}

fn global(bytes: &[u8]) -> Option<HGLOBAL> {
    unsafe {
        let g = GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1)).ok()?;
        let p = GlobalLock(g) as *mut u8;
        if p.is_null() {
            let _ = GlobalFree(Some(g));
            return None;
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
        let _ = GlobalUnlock(g);
        Some(g)
    }
}

/// Legt ein Format ab. Bei Erfolg gehört der Speicher dem System.
pub(crate) fn setzen(fmt: u32, bytes: &[u8]) -> bool {
    let Some(g) = global(bytes) else { return false };
    unsafe {
        if SetClipboardData(fmt, Some(HANDLE(g.0))).is_ok() {
            true
        } else {
            let _ = GlobalFree(Some(g));
            false
        }
    }
}

pub(crate) fn utf16_bytes(t: &str) -> Vec<u8> {
    t.encode_utf16()
        .chain(std::iter::once(0))
        .flat_map(|c| c.to_le_bytes())
        .collect()
}

/// Schreibt einen Eintrag in die Zwischenablage. `png` legt Bilder zusätzlich als PNG ab,
/// damit Programme, die das kennen, den Alphakanal behalten.
pub fn schreiben(hwnd: HWND, inhalt: &Inhalt, png: Option<&[u8]>) -> bool {
    let Some(_offen) = oeffnen(hwnd) else {
        return false;
    };
    if unsafe { EmptyClipboard() }.is_err() {
        return false;
    }
    match inhalt {
        Inhalt::Text(t) => setzen(CF_UNICODETEXT.0 as u32, &utf16_bytes(t)),
        Inhalt::Dateien(pfade) => {
            let ok = setzen(CF_HDROP.0 as u32, &formate::dropfiles_bauen(pfade));
            // DROPEFFECT_COPY: Explorer kopiert beim Einfügen, statt zu verschieben.
            setzen(format(w!("Preferred DropEffect")), &1u32.to_le_bytes());
            ok
        }
        Inhalt::Bild(b) => {
            let ok = setzen(CF_DIB.0 as u32, &formate::rgba_zu_dib(b));
            if let Some(p) = png {
                setzen(format(w!("PNG")), p);
            }
            ok
        }
    }
}

/// Dateiname der EXE, zu der ein Fenster gehört.
pub fn prozess_name(hwnd: HWND) -> Option<String> {
    if hwnd.is_invalid() {
        return None;
    }
    unsafe {
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            return None;
        }
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut puffer = [0u16; 1024];
        let mut laenge = puffer.len() as u32;
        let r = QueryFullProcessImageNameW(
            h,
            PROCESS_NAME_WIN32,
            PWSTR(puffer.as_mut_ptr()),
            &mut laenge,
        );
        let _ = CloseHandle(h);
        r.ok()?;
        let pfad = String::from_utf16_lossy(&puffer[..laenge as usize]);
        Some(pfad.rsplit('\\').next().unwrap_or(&pfad).to_string())
    }
}

/// Woher der aktuelle Inhalt kommt: Besitzer der Zwischenablage, sonst das Vordergrundfenster.
pub fn quelle() -> Option<String> {
    unsafe {
        GetClipboardOwner()
            .ok()
            .and_then(prozess_name)
            .or_else(|| prozess_name(GetForegroundWindow()))
    }
}

fn taste(vk: VIRTUAL_KEY, los: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: if los {
                    KEYEVENTF_KEYUP
                } else {
                    KEYBD_EVENT_FLAGS(0)
                },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

/// Drückt Strg+V im Vordergrundfenster.
pub fn strg_v() -> bool {
    let ein = [
        taste(VK_CONTROL, false),
        taste(VK_V, false),
        taste(VK_V, true),
        taste(VK_CONTROL, true),
    ];
    unsafe { SendInput(&ein, std::mem::size_of::<INPUT>() as i32) == ein.len() as u32 }
}

const RUN: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const WERT: PCWSTR = w!("Hikidashi");

pub fn autostart_an() -> bool {
    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            RUN,
            WERT,
            RRF_RT_REG_SZ,
            None,
            None,
            None,
        ) == ERROR_SUCCESS
    }
}

/// Trägt die aktuelle EXE in den Autostart ein oder entfernt sie.
pub fn autostart_setzen(an: bool) -> bool {
    unsafe {
        if an {
            let Ok(exe) = std::env::current_exe() else {
                return false;
            };
            let wert = utf16_bytes(&format!("\"{}\"", exe.display()));
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                RUN,
                WERT,
                REG_SZ.0,
                Some(wert.as_ptr() as *const c_void),
                wert.len() as u32,
            ) == ERROR_SUCCESS
        } else {
            RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN, WERT) == ERROR_SUCCESS
        }
    }
}
