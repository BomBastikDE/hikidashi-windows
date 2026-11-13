//! Das Programm: unsichtbares Fenster, das die Zwischenablage beobachtet, ein Symbol im
//! Infobereich und die Liste, die mit Strg+Alt+V an der Maus aufgeht.

use crate::ablage::{self, Inhalt};
use crate::formate;
use crate::verlauf::{self, Art, Reiter, Verlauf};
use std::cell::RefCell;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{
    GetLastError, ERROR_ALREADY_EXISTS, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    CreateFontIndirectW, GetMonitorInfoW, GetSysColor, GetSysColorBrush, InvalidateRect,
    MonitorFromPoint, SetBkColor, SetTextColor, COLOR_GRAYTEXT, COLOR_WINDOW, HDC, MONITORINFO,
    MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::System::DataExchange::{
    AddClipboardFormatListener, GetClipboardSequenceNumber,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::Controls::{EM_GETSEL, EM_SETCUEBANNER, EM_SETSEL};
use windows::Win32::UI::HiDpi::{
    GetDpiForSystem, SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_SYSTEM_AWARE,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, RegisterHotKey, SetFocus, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, VIRTUAL_KEY, VK_1,
    VK_9, VK_A, VK_C, VK_CONTROL, VK_DELETE, VK_DOWN, VK_ESCAPE, VK_F2, VK_NEXT, VK_P, VK_PRIOR,
    VK_RETURN, VK_SHIFT, VK_TAB, VK_UP, VK_V,
};
use windows::Win32::UI::Shell::{
    DefSubclassProc, SetWindowSubclass, Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE,
    NIF_TIP, NIIF_INFO, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW, NOTIFY_ICON_MESSAGE,
};
use windows::Win32::UI::WindowsAndMessaging::*;

const KLASSE: PCWSTR = w!("HikidashiFenster");
const WM_TRAY: u32 = WM_APP + 1;
const WM_ZEIGEN: u32 = WM_APP + 2;
const HOTKEY: i32 = 1;
const TIMER_EINFUEGEN: usize = 1;
const ID_SUCHE: usize = 101;
const ID_LISTE: usize = 102;

const M_OEFFNEN: usize = 1001;
const M_PAUSE: usize = 1002;
const M_AUTOSTART: usize = 1003;
const M_LEEREN: usize = 1004;
const M_ORDNER: usize = 1005;
const M_BEENDEN: usize = 1006;

const HILFE: &str = "Enter einfügen · Strg+P anheften · F2 Name · Entf löschen · Tab Reiter";
const HINWEIS_NAME: &str = "Name eingeben · Enter speichert · Esc bricht ab · leer = Inhalt zeigen";
const HINWEIS_NUR_ANGEHEFTET: &str = "Nur angeheftete Einträge bekommen einen Namen (erst Strg+P)";

struct Zustand {
    verlauf: Verlauf,
    reiter: Reiter,
    /// IDs der gerade angezeigten Zeilen.
    ids: Vec<u64>,
    pausiert: bool,
    /// Sequenznummer nach unserem eigenen Schreiben, damit wir uns nicht selbst aufnehmen.
    eigene_seq: u32,
    /// Fenster, in das eingefügt wird.
    vorher: HWND,
    fenster: HWND,
    kopf: HWND,
    suche: HWND,
    liste: HWND,
    fuss: HWND,
    symbol: HICON,
    taskbar_neu: u32,
    /// Eintrag, dessen Name gerade im Suchfeld bearbeitet wird (F2).
    umbenennen: Option<u64>,
}

thread_local! {
    static Z: RefCell<Option<Zustand>> = const { RefCell::new(None) };
}

/// Kurzer Zugriff auf den Zustand. Innerhalb von `f` keine Fensternachrichten auslösen,
/// sonst käme die Fensterprozedur in einen zweiten Zugriff.
fn mit<R>(f: impl FnOnce(&mut Zustand) -> R) -> Option<R> {
    Z.with(|z| z.try_borrow_mut().ok().and_then(|mut g| g.as_mut().map(f)))
}

fn jetzt() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn breit(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn melden(text: &str) {
    let t = breit(text);
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(t.as_ptr()),
            w!("Hikidashi"),
            MB_OK | MB_ICONINFORMATION,
        );
    }
}

fn fenstertext(h: HWND) -> String {
    unsafe {
        let n = GetWindowTextLengthW(h).max(0) as usize;
        let mut puffer = vec![0u16; n + 1];
        let gelesen = GetWindowTextW(h, &mut puffer).max(0) as usize;
        String::from_utf16_lossy(&puffer[..gelesen])
    }
}

fn text_setzen(h: HWND, s: &str) {
    let t = breit(s);
    unsafe {
        let _ = SetWindowTextW(h, PCWSTR(t.as_ptr()));
    }
}

pub fn datenordner() -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("Hikidashi")
}

fn kopieren(s: &str, ziel: &mut [u16]) {
    for (i, c) in s.encode_utf16().take(ziel.len() - 1).enumerate() {
        ziel[i] = c;
    }
}

fn tray(
    fenster: HWND,
    symbol: HICON,
    aktion: NOTIFY_ICON_MESSAGE,
    pausiert: bool,
    info: Option<&str>,
) {
    let mut nid = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: fenster,
        uID: 1,
        uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
        uCallbackMessage: WM_TRAY,
        hIcon: symbol,
        ..Default::default()
    };
    let tipp = if pausiert {
        "Hikidashi – pausiert"
    } else {
        "Hikidashi – Strg+Alt+V öffnet den Verlauf"
    };
    kopieren(tipp, &mut nid.szTip);
    if let Some(t) = info {
        nid.uFlags |= NIF_INFO;
        nid.dwInfoFlags = NIIF_INFO;
        kopieren(t, &mut nid.szInfo);
        kopieren("Hikidashi", &mut nid.szInfoTitle);
    }
    unsafe {
        let _ = Shell_NotifyIconW(aktion, &nid);
    }
}

fn symbol_erzeugen() -> HICON {
    unsafe {
        let g = GetSystemMetrics(SM_CXSMICON).max(16) as u32;
        let farbe = formate::symbol_bgra(g);
        let maske = vec![0u8; (g.div_ceil(16) * 2 * g) as usize];
        let inst = GetModuleHandleW(None).unwrap_or_default();
        CreateIcon(
            Some(inst.into()),
            g as i32,
            g as i32,
            1,
            32,
            maske.as_ptr(),
            farbe.as_ptr(),
        )
        .unwrap_or_else(|_| LoadIconW(None, IDI_APPLICATION).unwrap_or_default())
    }
}

pub fn starten() {
    unsafe {
        let _sperre = CreateMutexW(None, true, w!("Local\\Hikidashi-Zwischenablage"));
        if GetLastError() == ERROR_ALREADY_EXISTS {
            // Läuft schon: das vorhandene Fenster öffnen lassen.
            if let Ok(h) = FindWindowW(KLASSE, None) {
                let _ = PostMessageW(Some(h), WM_ZEIGEN, WPARAM(0), LPARAM(0));
            }
            return;
        }
        // Das Manifest setzt das bereits; für Builds ohne Manifest zusätzlich hier.
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_SYSTEM_AWARE);
        let skala = GetDpiForSystem() as f32 / 96.0;
        let px = |v: f32| (v * skala).round() as i32;

        let inst = GetModuleHandleW(None).unwrap_or_default();
        let wc = WNDCLASSW {
            lpfnWndProc: Some(fensterproc),
            hInstance: inst.into(),
            lpszClassName: KLASSE,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hbrBackground: GetSysColorBrush(COLOR_WINDOW),
            ..Default::default()
        };
        RegisterClassW(&wc);

        let (breite, hoehe) = (px(500.0), px(560.0));
        let Ok(fenster) = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
            KLASSE,
            w!("Hikidashi"),
            WS_POPUP | WS_BORDER,
            0,
            0,
            breite,
            hoehe,
            None,
            None,
            Some(inst.into()),
            None,
        ) else {
            melden("Das Fenster ließ sich nicht anlegen.");
            return;
        };

        let mut ncm = NONCLIENTMETRICSW {
            cbSize: std::mem::size_of::<NONCLIENTMETRICSW>() as u32,
            ..Default::default()
        };
        let _ = SystemParametersInfoW(
            SPI_GETNONCLIENTMETRICS,
            ncm.cbSize,
            Some(&mut ncm as *mut _ as *mut std::ffi::c_void),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        );
        let mut lf = ncm.lfMessageFont;
        lf.lfHeight = (lf.lfHeight as f32 * 1.15) as i32;
        let schrift = CreateFontIndirectW(&lf);

        let rand = px(10.0);
        let innen = breite - 2 * rand - 2;
        let (kopf_h, such_h, fuss_h) = (px(22.0), px(28.0), px(22.0));
        let kind =
            |klasse: PCWSTR, ex: WINDOW_EX_STYLE, stil: WINDOW_STYLE, y: i32, h: i32, id: usize| {
                let hwnd = CreateWindowExW(
                    ex,
                    klasse,
                    w!(""),
                    WS_CHILD | WS_VISIBLE | stil,
                    rand,
                    y,
                    innen,
                    h,
                    Some(fenster),
                    Some(HMENU(id as *mut _)),
                    Some(inst.into()),
                    None,
                )
                .unwrap_or_default();
                SendMessageW(
                    hwnd,
                    WM_SETFONT,
                    Some(WPARAM(schrift.0 as usize)),
                    Some(LPARAM(1)),
                );
                hwnd
            };
        let mut y = rand;
        let kopf = kind(
            w!("STATIC"),
            WINDOW_EX_STYLE(0),
            WINDOW_STYLE(0),
            y,
            kopf_h,
            0,
        );
        y += kopf_h + px(4.0);
        let suche = kind(
            w!("EDIT"),
            WS_EX_CLIENTEDGE,
            WS_TABSTOP | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
            y,
            such_h,
            ID_SUCHE,
        );
        y += such_h + px(6.0);
        let listen_h = hoehe - y - fuss_h - px(6.0) - rand;
        let liste = kind(
            w!("LISTBOX"),
            WS_EX_CLIENTEDGE,
            WS_VSCROLL | WINDOW_STYLE((LBS_NOTIFY | LBS_NOINTEGRALHEIGHT) as u32),
            y,
            listen_h,
            ID_LISTE,
        );
        y += listen_h + px(6.0);
        let fuss = kind(
            w!("STATIC"),
            WINDOW_EX_STYLE(0),
            WINDOW_STYLE(0),
            y,
            fuss_h,
            0,
        );
        text_setzen(fuss, HILFE);
        let hinweis = breit("Suchen …");
        SendMessageW(
            suche,
            EM_SETCUEBANNER,
            Some(WPARAM(1)),
            Some(LPARAM(hinweis.as_ptr() as isize)),
        );
        SendMessageW(
            liste,
            LB_SETITEMHEIGHT,
            Some(WPARAM(0)),
            Some(LPARAM(px(26.0) as isize)),
        );
        let _ = SetWindowSubclass(suche, Some(tastenproc), 1, 0);
        let _ = SetWindowSubclass(liste, Some(tastenproc), 1, 0);

        let ordner = datenordner();
        let erster_start = !ordner.join("verlauf.json").exists();
        let symbol = symbol_erzeugen();
        let mut verlauf = Verlauf::laden(&ordner);
        // Was eine ältere Version schon aufgenommen hat, nachträglich entfernen (Issue #3).
        if verlauf.texte_entfernen(formate::sieht_aus_wie_passwort) > 0 {
            let _ = verlauf.speichern();
        }
        Z.with(|z| {
            *z.borrow_mut() = Some(Zustand {
                verlauf,
                reiter: Reiter::Verlauf,
                ids: Vec::new(),
                pausiert: false,
                eigene_seq: 0,
                vorher: HWND::default(),
                fenster,
                kopf,
                suche,
                liste,
                fuss,
                symbol,
                taskbar_neu: RegisterWindowMessageW(w!("TaskbarCreated")),
                umbenennen: None,
            })
        });

        let _ = AddClipboardFormatListener(fenster);
        let taste_ok = RegisterHotKey(
            Some(fenster),
            HOTKEY,
            MOD_CONTROL | MOD_ALT | MOD_NOREPEAT,
            VK_V.0 as u32,
        )
        .is_ok();
        tray(
            fenster,
            symbol,
            NIM_ADD,
            false,
            erster_start.then_some("Läuft im Infobereich. Strg+Alt+V öffnet den Verlauf."),
        );
        if !taste_ok {
            melden(
                "Strg+Alt+V ist schon von einem anderen Programm belegt.\n\n\
                 Den Verlauf öffnest du mit einem Klick auf das Hikidashi-Symbol im Infobereich.",
            );
        }
        // Eintrag im Autostart auf den aktuellen Pfad der EXE nachziehen.
        if ablage::autostart_an() {
            ablage::autostart_setzen(true);
        }

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

fn sichtbar(h: HWND) -> bool {
    unsafe { IsWindowVisible(h).as_bool() }
}

fn verstecken() {
    if let Some(f) = mit(|z| z.fenster) {
        unsafe {
            let _ = ShowWindow(f, SW_HIDE);
        }
    }
}

fn zeigen() {
    let Some((f, suche)) = mit(|z| {
        let vorne = unsafe { GetForegroundWindow() };
        if vorne != z.fenster {
            z.vorher = vorne;
        }
        z.reiter = Reiter::Verlauf;
        z.umbenennen = None;
        (z.fenster, z.suche)
    }) else {
        return;
    };
    text_setzen(suche, "");
    aktualisieren(true);
    unsafe {
        let mut p = POINT::default();
        let _ = GetCursorPos(&mut p);
        let mut mi = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let _ = GetMonitorInfoW(MonitorFromPoint(p, MONITOR_DEFAULTTONEAREST), &mut mi);
        let mut r = RECT::default();
        let _ = GetWindowRect(f, &mut r);
        let (b, h) = (r.right - r.left, r.bottom - r.top);
        let a = mi.rcWork;
        let x = (p.x - b / 2).min(a.right - b).max(a.left);
        let y = (p.y - 20).min(a.bottom - h).max(a.top);
        let _ = SetWindowPos(
            f,
            Some(HWND_TOPMOST),
            x,
            y,
            0,
            0,
            SWP_NOSIZE | SWP_SHOWWINDOW,
        );
        let _ = SetForegroundWindow(f);
        let _ = SetFocus(Some(suche));
    }
}

/// Baut die Liste neu. `nach_oben` wählt die erste Zeile, sonst bleibt die Position.
fn aktualisieren(nach_oben: bool) {
    let Some((suche, liste, kopf)) = mit(|z| (z.suche, z.liste, z.kopf)) else {
        return;
    };
    let eingabe = fenstertext(suche);
    let Some((zeilen, kopftext)) = mit(|z| {
        // Beim Benennen steht der neue Name im Suchfeld: dann nicht filtern.
        let suchtext = if z.umbenennen.is_some() {
            ""
        } else {
            eingabe.as_str()
        };
        z.ids = z.verlauf.auswahl(z.reiter, suchtext);
        let t = jetzt();
        let zeilen: Vec<String> = z
            .ids
            .iter()
            .enumerate()
            .filter_map(|(i, id)| {
                z.verlauf
                    .finden(*id)
                    .map(|e| verlauf::anzeige(e, (i < 9).then_some(i + 1), t))
            })
            .collect();
        let mut kopf = Reiter::ALLE
            .iter()
            .map(|r| {
                if *r == z.reiter {
                    format!("[ {} ]", r.name())
                } else {
                    r.name().to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("    ");
        if z.umbenennen.is_some() {
            kopf = HINWEIS_NAME.to_string();
        } else if z.pausiert {
            kopf.push_str("     ⏸ pausiert");
        } else if zeilen.is_empty() {
            kopf.push_str(if suchtext.trim().is_empty() {
                "     (leer)"
            } else {
                "     nichts gefunden"
            });
        }
        (zeilen, kopf)
    }) else {
        return;
    };
    unsafe {
        let alt = SendMessageW(liste, LB_GETCURSEL, None, None).0;
        SendMessageW(liste, WM_SETREDRAW, Some(WPARAM(0)), None);
        SendMessageW(liste, LB_RESETCONTENT, None, None);
        for z in &zeilen {
            let t = breit(z);
            SendMessageW(liste, LB_ADDSTRING, None, Some(LPARAM(t.as_ptr() as isize)));
        }
        let n = zeilen.len() as isize;
        let neu = if n == 0 {
            -1
        } else if nach_oben || alt < 0 {
            0
        } else {
            alt.min(n - 1)
        };
        SendMessageW(liste, LB_SETCURSEL, Some(WPARAM(neu as usize)), None);
        SendMessageW(liste, WM_SETREDRAW, Some(WPARAM(1)), None);
        let _ = InvalidateRect(Some(liste), None, true);
    }
    text_setzen(kopf, &kopftext);
}

/// Wählt die Zeile mit diesem Eintrag aus, falls sie sichtbar ist.
fn auswaehlen(id: u64) {
    let Some((liste, Some(i))) = mit(|z| (z.liste, z.ids.iter().position(|x| *x == id))) else {
        return;
    };
    unsafe {
        SendMessageW(liste, LB_SETCURSEL, Some(WPARAM(i)), None);
    }
}

/// F2: Namen des ausgewählten angehefteten Eintrags im Suchfeld bearbeiten.
fn benennen_starten() {
    let Some(id) = ausgewaehlt() else { return };
    let Some((suche, kopf, start)) = mit(|z| {
        let e = z.verlauf.finden(id)?;
        if !e.angeheftet {
            return Some((z.suche, z.kopf, None));
        }
        let vorschlag = e.titel.clone().unwrap_or_else(|| {
            e.text
                .as_deref()
                .map(verlauf::erste_zeile)
                .unwrap_or_default()
        });
        z.umbenennen = Some(id);
        Some((z.suche, z.kopf, Some(vorschlag)))
    })
    .flatten() else {
        return;
    };
    let Some(vorschlag) = start else {
        text_setzen(kopf, HINWEIS_NUR_ANGEHEFTET);
        return;
    };
    // Erst den Modus setzen (oben), dann das Feld füllen: EN_CHANGE filtert dann nicht.
    text_setzen(suche, &vorschlag);
    aktualisieren(false);
    auswaehlen(id);
    unsafe {
        let _ = SetFocus(Some(suche));
        SendMessageW(suche, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
    }
}

/// Enter (speichern) oder Esc (abbrechen) beim Benennen.
fn benennen_beenden(speichern: bool) {
    let Some((suche, id)) = mit(|z| (z.suche, z.umbenennen.take())) else {
        return;
    };
    let Some(id) = id else { return };
    if speichern {
        let name = fenstertext(suche);
        mit(|z| {
            if z.verlauf.umbenennen(id, &name) {
                let _ = z.verlauf.speichern();
            }
        });
    }
    // Modus ist aus: Leeren des Feldes baut die Liste ungefiltert neu auf.
    text_setzen(suche, "");
    aktualisieren(false);
    auswaehlen(id);
}

fn ausgewaehlt() -> Option<u64> {
    let liste = mit(|z| z.liste)?;
    let i = unsafe { SendMessageW(liste, LB_GETCURSEL, None, None).0 };
    if i < 0 {
        return None;
    }
    mit(|z| z.ids.get(i as usize).copied()).flatten()
}

fn bewegen(liste: HWND, schritt: isize) {
    unsafe {
        let n = SendMessageW(liste, LB_GETCOUNT, None, None).0;
        if n <= 0 {
            return;
        }
        let i = SendMessageW(liste, LB_GETCURSEL, None, None).0;
        let neu = (i + schritt).clamp(0, n - 1);
        SendMessageW(liste, LB_SETCURSEL, Some(WPARAM(neu as usize)), None);
    }
}

/// Legt den Eintrag in die Zwischenablage und fügt ihn, wenn gewünscht, im vorigen Fenster ein.
fn einfuegen(id: Option<u64>, nur_kopieren: bool) {
    let Some(id) = id else { return };
    let Some((fenster, inhalt, png, vorher)) = mit(|z| {
        let e = z.verlauf.finden(id)?.clone();
        let (inhalt, png) = match e.art {
            Art::Text => (Inhalt::Text(e.text.clone()?), None),
            Art::Datei => (Inhalt::Dateien(e.dateien.clone()), None),
            Art::Bild => {
                let p = std::fs::read(z.verlauf.bild_pfad(&e)?).ok()?;
                let b = formate::png_dekodieren(&p).ok()?;
                (Inhalt::Bild(b), Some(p))
            }
        };
        Some((z.fenster, inhalt, png, z.vorher))
    })
    .flatten() else {
        return;
    };
    let ok = ablage::schreiben(fenster, &inhalt, png.as_deref());
    mit(|z| {
        z.eigene_seq = unsafe { GetClipboardSequenceNumber() };
        if ok {
            z.verlauf.benutzt(id, jetzt());
            let _ = z.verlauf.speichern();
        }
    });
    unsafe {
        let _ = ShowWindow(fenster, SW_HIDE);
        if ok && !nur_kopieren && !vorher.is_invalid() {
            let _ = SetForegroundWindow(vorher);
            // Kurz warten, bis das Zielfenster den Fokus hat.
            SetTimer(Some(fenster), TIMER_EINFUEGEN, 60, None);
        }
    }
}

/// Neuer Inhalt in der Zwischenablage.
fn aufnehmen(h: HWND) {
    let seq = unsafe { GetClipboardSequenceNumber() };
    if mit(|z| !z.pausiert && seq != z.eigene_seq) != Some(true) {
        return;
    }
    let quelle = ablage::quelle();
    if quelle.as_deref().is_some_and(formate::ist_passwortmanager) {
        return;
    }
    let Some(inhalt) = ablage::lesen(h) else {
        return;
    };
    // Passwörter aus Browser-Erweiterungen tragen keine Markierung: am Inhalt erkennen.
    if matches!(&inhalt, Inhalt::Text(t) if formate::sieht_aus_wie_passwort(t)) {
        return;
    }
    // Kodieren außerhalb des Zustands; bei großen Bildern dauert das einen Moment.
    let bild = match &inhalt {
        Inhalt::Bild(b) => match formate::png_kodieren(b) {
            Ok(p) => Some((p, formate::pruefsumme(&b.pixel))),
            Err(_) => return,
        },
        _ => None,
    };
    let offen = mit(|z| {
        let t = jetzt();
        match inhalt {
            Inhalt::Text(s) => {
                z.verlauf.text_aufnehmen(s, quelle, t);
            }
            Inhalt::Dateien(d) => {
                z.verlauf.dateien_aufnehmen(d, quelle, t);
            }
            Inhalt::Bild(b) => {
                if let Some((png, summe)) = bild {
                    let _ = z
                        .verlauf
                        .bild_aufnehmen(&png, b.breite, b.hoehe, summe, quelle, t);
                }
            }
        }
        let _ = z.verlauf.speichern();
        sichtbar(z.fenster)
    });
    if offen == Some(true) {
        aktualisieren(false);
    }
}

fn menue() {
    let Some((fenster, pausiert)) = mit(|z| (z.fenster, z.pausiert)) else {
        return;
    };
    let haken = |an: bool| if an { MF_CHECKED } else { MF_UNCHECKED };
    unsafe {
        let Ok(m) = CreatePopupMenu() else { return };
        let _ = AppendMenuW(m, MF_STRING, M_OEFFNEN, w!("Verlauf öffnen\tStrg+Alt+V"));
        let _ = AppendMenuW(m, MF_SEPARATOR, 0, None);
        let _ = AppendMenuW(
            m,
            MF_STRING | haken(pausiert),
            M_PAUSE,
            w!("Aufnahme pausieren"),
        );
        let _ = AppendMenuW(
            m,
            MF_STRING | haken(ablage::autostart_an()),
            M_AUTOSTART,
            w!("Mit Windows starten"),
        );
        let _ = AppendMenuW(m, MF_STRING, M_ORDNER, w!("Datenordner öffnen"));
        let _ = AppendMenuW(m, MF_STRING, M_LEEREN, w!("Verlauf leeren …"));
        let _ = AppendMenuW(m, MF_SEPARATOR, 0, None);
        let _ = AppendMenuW(m, MF_STRING, M_BEENDEN, w!("Beenden"));
        let mut p = POINT::default();
        let _ = GetCursorPos(&mut p);
        // Ohne Vordergrund schließt das Menü nicht beim Klick daneben.
        let _ = SetForegroundWindow(fenster);
        let wahl = TrackPopupMenu(
            m,
            TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN,
            p.x,
            p.y,
            None,
            fenster,
            None,
        );
        let _ = PostMessageW(Some(fenster), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(m);
        match wahl.0 as usize {
            M_OEFFNEN => zeigen(),
            M_PAUSE => {
                if let Some((f, s, an)) = mit(|z| {
                    z.pausiert = !z.pausiert;
                    (z.fenster, z.symbol, z.pausiert)
                }) {
                    tray(f, s, NIM_MODIFY, an, None);
                }
            }
            M_AUTOSTART => {
                let an = !ablage::autostart_an();
                if !ablage::autostart_setzen(an) {
                    melden("Der Autostart ließ sich nicht ändern.");
                }
            }
            M_ORDNER => {
                if let Some(o) = mit(|z| z.verlauf.ordner().to_path_buf()) {
                    let _ = std::process::Command::new("explorer.exe").arg(o).spawn();
                }
            }
            M_LEEREN => {
                let antwort = MessageBoxW(
                    None,
                    w!("Den ganzen Verlauf löschen? Angeheftete Einträge bleiben."),
                    w!("Hikidashi"),
                    MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2,
                );
                if antwort == IDYES {
                    mit(|z| {
                        z.verlauf.leeren();
                        let _ = z.verlauf.speichern();
                    });
                }
            }
            M_BEENDEN => {
                let _ = DestroyWindow(fenster);
            }
            _ => {}
        }
    }
}

fn niedrig(w: usize) -> usize {
    w & 0xffff
}

unsafe extern "system" fn fensterproc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_CLIPBOARDUPDATE => {
            aufnehmen(h);
            LRESULT(0)
        }
        WM_HOTKEY => {
            if sichtbar(h) {
                verstecken();
            } else {
                zeigen();
            }
            LRESULT(0)
        }
        WM_ZEIGEN => {
            zeigen();
            LRESULT(0)
        }
        WM_TRAY => {
            match niedrig(lp.0 as usize) as u32 {
                WM_LBUTTONUP => zeigen(),
                WM_RBUTTONUP => menue(),
                _ => {}
            }
            LRESULT(0)
        }
        WM_ACTIVATE => {
            if niedrig(wp.0) as u32 == WA_INACTIVE {
                verstecken();
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            let (id, code) = (niedrig(wp.0), (wp.0 >> 16) & 0xffff);
            if id == ID_SUCHE && code == EN_CHANGE as usize {
                if mit(|z| z.umbenennen.is_none()) == Some(true) {
                    aktualisieren(true);
                }
            } else if id == ID_LISTE && code == LBN_DBLCLK as usize {
                einfuegen(ausgewaehlt(), false);
            }
            LRESULT(0)
        }
        WM_TIMER if wp.0 == TIMER_EINFUEGEN => {
            let _ = KillTimer(Some(h), TIMER_EINFUEGEN);
            ablage::strg_v();
            LRESULT(0)
        }
        WM_CTLCOLORSTATIC => {
            let hdc = HDC(wp.0 as *mut _);
            let ist_fuss = mit(|z| z.fuss.0 as isize == lp.0) == Some(true);
            SetBkColor(
                hdc,
                windows::Win32::Foundation::COLORREF(GetSysColor(COLOR_WINDOW)),
            );
            if ist_fuss {
                SetTextColor(
                    hdc,
                    windows::Win32::Foundation::COLORREF(GetSysColor(COLOR_GRAYTEXT)),
                );
            }
            LRESULT(GetSysColorBrush(COLOR_WINDOW).0 as isize)
        }
        WM_DESTROY => {
            if let Some((f, s)) = mit(|z| (z.fenster, z.symbol)) {
                tray(f, s, NIM_DELETE, false, None);
            }
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => {
            if mit(|z| z.taskbar_neu == msg && msg != 0) == Some(true) {
                // Explorer wurde neu gestartet: Symbol wieder anlegen.
                if let Some((f, s, p)) = mit(|z| (z.fenster, z.symbol, z.pausiert)) {
                    tray(f, s, NIM_ADD, p, None);
                }
                return LRESULT(0);
            }
            DefWindowProcW(h, msg, wp, lp)
        }
    }
}

fn gedrueckt(vk: VIRTUAL_KEY) -> bool {
    unsafe { GetKeyState(vk.0 as i32) < 0 }
}

/// Tasten im Suchfeld und in der Liste.
unsafe extern "system" fn tastenproc(
    h: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    _id: usize,
    _daten: usize,
) -> LRESULT {
    let Some((suche, liste)) = mit(|z| (z.suche, z.liste)) else {
        return DefSubclassProc(h, msg, wp, lp);
    };
    let strg = gedrueckt(VK_CONTROL);
    match msg {
        WM_KEYDOWN => {
            let vk = VIRTUAL_KEY(wp.0 as u16);
            if mit(|z| z.umbenennen.is_some()) == Some(true) {
                // Beim Benennen: nur Enter/Esc; Navigation und Kürzel ruhen,
                // normale Bearbeitung (Entf, Pfeile links/rechts, Strg+C/V) bleibt im Feld.
                if vk == VK_RETURN || vk == VK_ESCAPE {
                    benennen_beenden(vk == VK_RETURN);
                    return LRESULT(0);
                }
                let gesperrt = [VK_DOWN, VK_UP, VK_PRIOR, VK_NEXT, VK_TAB, VK_F2].contains(&vk)
                    || (h == liste && vk == VK_DELETE)
                    || (strg && (vk == VK_P || (VK_1.0..=VK_9.0).contains(&vk.0)));
                if gesperrt {
                    return LRESULT(0);
                }
                if strg && vk == VK_A && h == suche {
                    SendMessageW(h, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
                    return LRESULT(0);
                }
                return DefSubclassProc(h, msg, wp, lp);
            }
            if vk == VK_F2 {
                benennen_starten();
                return LRESULT(0);
            }
            if h == suche && (vk == VK_DOWN || vk == VK_UP || vk == VK_PRIOR || vk == VK_NEXT) {
                let schritt = match vk {
                    VK_DOWN => 1,
                    VK_UP => -1,
                    VK_NEXT => 10,
                    _ => -10,
                };
                bewegen(liste, schritt);
                return LRESULT(0);
            }
            if vk == VK_RETURN {
                einfuegen(ausgewaehlt(), strg);
                return LRESULT(0);
            }
            if vk == VK_ESCAPE {
                verstecken();
                return LRESULT(0);
            }
            if vk == VK_TAB {
                let rueck = gedrueckt(VK_SHIFT);
                mit(|z| z.reiter = z.reiter.naechster(rueck));
                aktualisieren(true);
                return LRESULT(0);
            }
            if vk == VK_DELETE && (h == liste || fenstertext(suche).is_empty()) {
                if let Some(id) = ausgewaehlt() {
                    mit(|z| {
                        z.verlauf.loeschen(id);
                        let _ = z.verlauf.speichern();
                    });
                    aktualisieren(false);
                }
                return LRESULT(0);
            }
            if strg && vk == VK_P {
                if let Some(id) = ausgewaehlt() {
                    mit(|z| {
                        z.verlauf.anheften_umschalten(id);
                        let _ = z.verlauf.speichern();
                    });
                    aktualisieren(false);
                }
                return LRESULT(0);
            }
            if strg && (VK_1.0..=VK_9.0).contains(&vk.0) {
                let n = (vk.0 - VK_1.0) as usize;
                einfuegen(mit(|z| z.ids.get(n).copied()).flatten(), false);
                return LRESULT(0);
            }
            if strg && vk == VK_A && h == suche {
                SendMessageW(h, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
                return LRESULT(0);
            }
            if strg && vk == VK_C {
                let mut von = 0u32;
                let mut bis = 0u32;
                if h == suche {
                    SendMessageW(
                        h,
                        EM_GETSEL,
                        Some(WPARAM(&mut von as *mut u32 as usize)),
                        Some(LPARAM(&mut bis as *mut u32 as isize)),
                    );
                }
                if h == liste || von == bis {
                    einfuegen(ausgewaehlt(), true);
                    return LRESULT(0);
                }
            }
        }
        WM_CHAR => {
            let c = wp.0 as u32;
            // Enter, Strg+Enter, Esc, Tab, Strg+A, Strg+P: kein Piepen, kein Steuerzeichen im Feld.
            if matches!(c, 0x0D | 0x0A | 0x1B | 0x09 | 0x01 | 0x10) {
                return LRESULT(0);
            }
            // Tippen in der Liste geht ins Suchfeld.
            if h == liste && (c >= 0x20 || c == 0x08) {
                let _ = SetFocus(Some(suche));
                SendMessageW(suche, WM_CHAR, Some(wp), Some(lp));
                return LRESULT(0);
            }
        }
        _ => {}
    }
    DefSubclassProc(h, msg, wp, lp)
}
