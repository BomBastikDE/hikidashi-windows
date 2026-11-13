# Änderungen

## 0.2.0 — unveröffentlicht

- **Angeheftetes benennen:** F2 gibt einem angehefteten Eintrag einen eigenen Namen,
  der in der Liste statt des Inhalts steht. Eingefügt wird weiter der Inhalt; die
  Suche findet Name und Inhalt. Leerer Name zeigt wieder den Inhalt (#6).

## 0.1.0 — 2026-11-13

Erste Windows-Fassung, neu geschrieben in Rust nach dem Vorbild von
[Hikidashi für den Mac](https://github.com/Kenvoice/hikidashi) 1.0.0.

- Verlauf für Text, Bilder und Dateien aus dem Explorer, mit Suche.
  Strg+Alt+V öffnet ihn an der Maus, Enter fügt direkt ein, Strg+1 bis Strg+9
  fügen sofort ein.
- Anheften, Löschen, Reiter Verlauf / Angeheftet / Bilder.
- Symbol im Infobereich mit Pausieren, Autostart, Datenordner und Verlauf leeren.
- Passwort-Manager und als geheim markierte Inhalte werden nicht aufgenommen.
- Passwortähnlicher Text wird aus allen Programmen nicht aufgenommen, damit auch
  Passwörter aus Browser-Erweiterungen nicht im Verlauf landen (#3). Bereits
  gespeicherte, nicht angeheftete Einträge dieser Art entfernt der nächste Start.
- `--selbsttest` prüft Zwischenablage, Einfügen und Autostart unter Windows.

Praxistest unter Windows bestanden (#1), inklusive Skalierung 125 %/150 % und
Passwörtern aus der NordPass-Erweiterung in Chrome.

Noch nicht enthalten: Texterkennung auf Bildern, Inbox-Ordner, Bildvorschau.
