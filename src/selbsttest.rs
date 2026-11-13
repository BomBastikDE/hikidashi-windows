//! `hikidashi.exe --selbsttest`: prüft die Windows-Teile an der echten Zwischenablage.
//! Läuft in der CI auf einem Windows-Runner. Ausgabe auf stdout, Rückgabe 0 = alles gut.

use crate::ablage::{self, Inhalt};
use crate::formate::Rgba;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};
use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
use windows::Win32::System::DataExchange::{AddClipboardFormatListener, EmptyClipboard};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Ole::CF_UNICODETEXT;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, SetFocus, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS,
    KEYEVENTF_KEYUP, VK_MENU,
};
use windows::Win32::UI::WindowsAndMessaging::*;

static AENDERUNGEN: AtomicU32 = AtomicU32::new(0);

unsafe extern "system" fn testproc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_CLIPBOARDUPDATE {
        AENDERUNGEN.fetch_add(1, Ordering::SeqCst);
        return LRESULT(0);
    }
    DefWindowProcW(h, msg, wp, lp)
}

fn pumpen(ms: u64) {
    let ende = Instant::now() + Duration::from_millis(ms);
    unsafe {
        let mut msg = MSG::default();
        while Instant::now() < ende {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

fn text_lesen(h: HWND) -> Option<String> {
    match ablage::lesen(h) {
        Some(Inhalt::Text(t)) => Some(t),
        _ => None,
    }
}

/// Holt ein eigenes Fenster nach vorn; auf Runnern hilft ein kurzer Alt-Tastendruck.
fn nach_vorn(h: HWND) -> bool {
    unsafe {
        for _ in 0..5 {
            let _ = SetForegroundWindow(h);
            pumpen(100);
            if GetForegroundWindow() == h {
                return true;
            }
            let alt = |los: bool| INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: VK_MENU,
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
            };
            SendInput(
                &[alt(false), alt(true)],
                std::mem::size_of::<INPUT>() as i32,
            );
        }
        GetForegroundWindow() == h
    }
}

pub fn ausfuehren() -> i32 {
    unsafe {
        let _ = AttachConsole(ATTACH_PARENT_PROCESS);
    }
    let mut fehler = 0;
    let mut pruefen = |name: &str, ok: bool| {
        println!("{} {name}", if ok { "OK    " } else { "FEHLER" });
        if !ok {
            fehler += 1;
        }
    };

    unsafe {
        let inst = GetModuleHandleW(None).unwrap_or_default();
        let wc = WNDCLASSW {
            lpfnWndProc: Some(testproc),
            hInstance: inst.into(),
            lpszClassName: w!("HikidashiTest"),
            ..Default::default()
        };
        RegisterClassW(&wc);
        let fenster = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("HikidashiTest"),
            w!("Hikidashi Selbsttest"),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            100,
            100,
            500,
            200,
            None,
            None,
            Some(inst.into()),
            None,
        )
        .expect("Testfenster");
        let feld = CreateWindowExW(
            WS_EX_CLIENTEDGE,
            w!("EDIT"),
            w!(""),
            WS_CHILD | WS_VISIBLE | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
            10,
            10,
            400,
            30,
            Some(fenster),
            None,
            Some(inst.into()),
            None,
        )
        .expect("Eingabefeld");

        // 1. Text hin und zurück, mit Umlauten und Zeichen außerhalb der BMP.
        let text = "Hikidashi Selbsttest äöü ß 🍣 引き出し";
        pruefen(
            "Text schreiben und lesen",
            ablage::schreiben(fenster, &Inhalt::Text(text.into()), None)
                && text_lesen(fenster).as_deref() == Some(text),
        );

        // 2. Dateiliste.
        let pfade = vec![
            r"C:\Windows\notepad.exe".to_string(),
            r"C:\Temp\Bericht ä.pdf".to_string(),
        ];
        let dateien_ok = ablage::schreiben(fenster, &Inhalt::Dateien(pfade.clone()), None)
            && matches!(ablage::lesen(fenster), Some(Inhalt::Dateien(p)) if p == pfade);
        pruefen("Dateiliste schreiben und lesen", dateien_ok);

        // 3. Bild als DIB.
        let bild = Rgba {
            breite: 3,
            hoehe: 2,
            pixel: vec![
                255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 10, 20, 30, 255, 40, 50, 60, 255,
                70, 80, 90, 255,
            ],
        };
        let bild_ok = ablage::schreiben(
            fenster,
            &Inhalt::Bild(Rgba {
                pixel: bild.pixel.clone(),
                ..bild
            }),
            None,
        ) && matches!(ablage::lesen(fenster), Some(Inhalt::Bild(b)) if b.breite == 3 && b.hoehe == 2 && b.pixel == bild.pixel);
        pruefen("Bild schreiben und lesen", bild_ok);

        // 4. Als geheim markierte Inhalte werden nicht gelesen.
        let geheim = {
            let offen = ablage::oeffnen(fenster);
            let ok = offen.is_some()
                && EmptyClipboard().is_ok()
                && ablage::setzen(CF_UNICODETEXT.0 as u32, &ablage::utf16_bytes("Passwort123"))
                && ablage::setzen(
                    ablage::format(w!("ExcludeClipboardContentFromMonitorProcessing")),
                    &[0u8; 4],
                );
            drop(offen);
            ok && ablage::lesen(fenster).is_none()
        };
        pruefen("Markierte Passwörter werden übersprungen", geheim);
        let verlauf_aus = {
            let offen = ablage::oeffnen(fenster);
            let ok = offen.is_some()
                && EmptyClipboard().is_ok()
                && ablage::setzen(
                    CF_UNICODETEXT.0 as u32,
                    &ablage::utf16_bytes("Nicht in den Verlauf"),
                )
                && ablage::setzen(
                    ablage::format(w!("CanIncludeInClipboardHistory")),
                    &0u32.to_le_bytes(),
                );
            drop(offen);
            ok && ablage::lesen(fenster).is_none()
        };
        pruefen("„Nicht in den Verlauf“ wird beachtet", verlauf_aus);

        // 5. Benachrichtigung bei Änderungen.
        let _ = AddClipboardFormatListener(fenster);
        pumpen(100);
        AENDERUNGEN.store(0, Ordering::SeqCst);
        ablage::schreiben(fenster, &Inhalt::Text("Änderung".into()), None);
        pumpen(300);
        pruefen(
            "Benachrichtigung bei neuem Inhalt",
            AENDERUNGEN.load(Ordering::SeqCst) > 0,
        );

        // 6. Einfügen per Strg+V in ein fremdes Eingabefeld (hier unser eigenes).
        let ziel = "Einfügetest 42 ✓";
        ablage::schreiben(fenster, &Inhalt::Text(ziel.into()), None);
        let vorn = nach_vorn(fenster);
        let _ = SetFocus(Some(feld));
        pumpen(100);
        ablage::strg_v();
        pumpen(500);
        let mut puffer = [0u16; 256];
        let n = GetWindowTextW(feld, &mut puffer).max(0) as usize;
        let eingefuegt = String::from_utf16_lossy(&puffer[..n]);
        if !vorn {
            println!("       (Testfenster kam nicht in den Vordergrund)");
        }
        pruefen("Einfügen mit Strg+V", eingefuegt == ziel);

        // 7. Autostart-Eintrag setzen, lesen und wieder entfernen (nur, wenn vorher keiner da war).
        if !ablage::autostart_an() {
            let an = ablage::autostart_setzen(true) && ablage::autostart_an();
            let aus = ablage::autostart_setzen(false) && !ablage::autostart_an();
            pruefen("Autostart ein- und ausschalten", an && aus);
        } else {
            println!("       Autostart übersprungen (schon eingerichtet)");
        }

        let _ = DestroyWindow(fenster);
    }
    println!("{fehler} Fehler");
    if fehler == 0 {
        0
    } else {
        1
    }
}
