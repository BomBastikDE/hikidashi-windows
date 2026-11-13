#![cfg_attr(all(windows, not(test)), windows_subsystem = "windows")]
#![cfg_attr(not(windows), allow(dead_code))]
#![allow(clippy::chunks_exact_to_as_chunks)]

mod formate;
mod verlauf;

#[cfg(windows)]
mod ablage;
#[cfg(windows)]
mod app;
#[cfg(windows)]
mod selbsttest;

#[cfg(windows)]
fn main() {
    if std::env::args().any(|a| a == "--selbsttest") {
        std::process::exit(selbsttest::ausfuehren());
    }
    app::starten();
}

#[cfg(not(windows))]
fn main() {
    eprintln!("Hikidashi läuft nur unter Windows. Hier laufen nur die Tests: cargo test");
}
