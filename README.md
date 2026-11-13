# Hikidashi für Windows 引き出し

Verlauf der Zwischenablage für Windows. Alles, was du kopierst, landet in einer
Liste: Text, Bilder und Screenshots, Dateien aus dem Explorer. Mit **Strg+Alt+V**
öffnest du die Liste an der Maus, tippst ein Wort und fügst den Eintrag mit Enter
direkt ein. 引き出し (hikidashi) heißt „Schublade“.

Windows-Fassung von [Hikidashi für den Mac](https://github.com/Kenvoice/hikidashi)
von Kenzo Weiss. Neu geschrieben in Rust; Bedienung, Grenzen und Datenformat folgen
dem Original.

> **Stand 0.1:** erste Version, unter Windows noch nicht von Hand getestet. Die CI
> prüft Zwischenablage, Einfügen und Autostart auf einem Windows-Runner.
> Texterkennung auf Bildern und der Inbox-Ordner kommen in Schritt 2.

## Was die App kann

- **Verlauf:** neueste Einträge oben, sofort durchsuchbar (alle Wörter müssen
  passen, auch der Name des Programms, aus dem kopiert wurde).
- **Einfügen:** Enter fügt in das Fenster ein, in dem du gerade warst.
  Strg+1 bis Strg+9 fügen den Eintrag mit dieser Nummer sofort ein.
- **Bilder** und **Dateien** aus dem Explorer kommen mit in den Verlauf und lassen
  sich wieder einfügen.
- **Anheften:** Strg+P hält einen Eintrag fest. Angeheftetes bleibt, auch wenn du
  den Verlauf leerst.
- **Pausieren:** nimmt nichts auf, solange du mit vertraulichen Daten arbeitest.
- **Mit Windows starten:** im Menü des Symbols im Infobereich.

| Taste | Was passiert |
|---|---|
| Strg+Alt+V | Verlauf öffnen oder schließen |
| Enter | einfügen |
| Strg+Enter oder Strg+C | nur kopieren |
| Strg+1 bis Strg+9 | Eintrag mit dieser Nummer einfügen |
| Strg+P | anheften oder lösen |
| Entf | Eintrag löschen (in der Liste oder bei leerer Suche) |
| Tab / Umschalt+Tab | Reiter wechseln: Verlauf, Angeheftet, Bilder |
| Esc | schließen |

Linksklick auf das Symbol im Infobereich öffnet den Verlauf, Rechtsklick das Menü.

## Deine Daten bleiben auf dem Rechner

Hikidashi hat keinen einzigen Netzaufruf. Der Verlauf liegt unter
`%APPDATA%\Hikidashi\`:

| Datei | Inhalt |
|---|---|
| `verlauf.json` | alle Einträge mit Text, Herkunft und Zeit |
| `bilder\` | die kopierten Bilder als PNG |

Das steht im Klartext in deinem Benutzerordner. Wer mit sensiblen Daten arbeitet,
sollte die Platte verschlüsseln (BitLocker) und den Verlauf ab und zu leeren.

Nicht aufgenommen wird:

- alles, was Programme als geheim oder „nicht in den Verlauf“ markieren
  (`ExcludeClipboardContentFromMonitorProcessing`, `CanIncludeInClipboardHistory = 0`),
  und alles aus 1Password, Bitwarden, KeePass, KeePassXC, Dashlane, LastPass, Enpass,
  Proton Pass, NordPass, RoboForm und Keeper
- Text, der wie ein Passwort aussieht – egal aus welchem Programm. Das ist nötig, weil
  Browser-Erweiterungen von Passwort-Managern (NordPass, Bitwarden & Co. in Chrome
  oder Edge) beim Kopieren keine Markierung setzen. Als passwortähnlich gilt ein
  einzelnes Wort mit 8 bis 64 Zeichen aus mindestens drei Zeichenarten (Klein-,
  Großbuchstaben, Ziffern, Sonderzeichen). Webadressen, E-Mail-Adressen, Pfade, UUIDs
  und Datumsangaben zählen nicht dazu. Nebenwirkung: Auch API-Schlüssel und ähnliche
  Codes landen nicht im Verlauf.
- Text über 2 MB und Bilder über 40 MB

Der Verlauf hält 300 Einträge, davon höchstens 60 Bilder. Das Älteste fällt raus,
Angeheftetes zählt nicht mit.

## Installieren

Die fertige `hikidashi.exe` liegt nach jedem Lauf der CI als Artefakt
`hikidashi-windows-x64` unter *Actions* (mit SHA256-Prüfsumme). Die Datei irgendwo
ablegen, z. B. unter `%LOCALAPPDATA%\Programs\Hikidashi\`, und starten. Es gibt
keinen Installer. Die EXE ist nicht signiert, deshalb warnt SmartScreen beim ersten
Start („Weitere Informationen“ → „Trotzdem ausführen“).

## Selbst bauen

Rust 1.99 oder neuer mit dem MSVC-Toolchain (Visual Studio Build Tools):

```
cargo build --release
target\release\hikidashi.exe --selbsttest
```

Die Logik für Verlauf, Bild- und Dateiformate ist plattformunabhängig und lässt
sich auch unter Linux testen: `cargo test`.

| Datei | Aufgabe |
|---|---|
| `src/verlauf.rs` | Einträge, Grenzen, Suche, Speichern (plattformunabhängig) |
| `src/formate.rs` | DIB, Dateilisten, PNG, Passwort-Manager, Symbol (plattformunabhängig) |
| `src/ablage.rs` | Windows-Zwischenablage, Strg+V, Autostart |
| `src/app.rs` | Fenster, Tastenkürzel, Infobereich |
| `src/selbsttest.rs` | Prüfung an der echten Zwischenablage (`--selbsttest`) |

## Lizenz

MIT, siehe [LICENSE](LICENSE). Idee, Bedienkonzept und Name stammen aus dem
Mac-Original von Kenzo Weiss.
