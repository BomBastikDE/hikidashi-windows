//! Der Verlauf: Einträge aufnehmen, kürzen, anheften, suchen und speichern.
//! Plattformunabhängig, damit die Logik auch unter Linux getestet werden kann.

use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// So viele nicht angeheftete Einträge hält der Verlauf.
pub const MAX_EINTRAEGE: usize = 300;
/// Davon höchstens so viele Bilder.
pub const MAX_BILDER: usize = 60;
/// Größere Texte werden nicht aufgenommen.
pub const MAX_TEXT_BYTES: usize = 2 * 1024 * 1024;
/// Größere Bilder (Rohdaten RGBA) werden nicht aufgenommen.
pub const MAX_BILD_BYTES: usize = 40 * 1024 * 1024;

const DATEI: &str = "verlauf.json";
const BILDER: &str = "bilder";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Art {
    Text,
    Bild,
    Datei,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Eintrag {
    pub id: u64,
    pub art: Art,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dateien: Vec<String>,
    /// Dateiname des Bildes im Ordner `bilder/`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bild: Option<String>,
    #[serde(default)]
    pub breite: u32,
    #[serde(default)]
    pub hoehe: u32,
    #[serde(default)]
    pub pruefsumme: u64,
    /// Programm, aus dem kopiert wurde (Dateiname der EXE).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quelle: Option<String>,
    /// Unix-Zeit in Sekunden.
    pub zeit: i64,
    #[serde(default)]
    pub angeheftet: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reiter {
    Verlauf,
    Angeheftet,
    Bilder,
}

impl Reiter {
    pub const ALLE: [Reiter; 3] = [Reiter::Verlauf, Reiter::Angeheftet, Reiter::Bilder];

    pub fn name(self) -> &'static str {
        match self {
            Reiter::Verlauf => "Verlauf",
            Reiter::Angeheftet => "Angeheftet",
            Reiter::Bilder => "Bilder",
        }
    }

    pub fn naechster(self, rueckwaerts: bool) -> Reiter {
        let i = Self::ALLE.iter().position(|r| *r == self).unwrap_or(0);
        let n = Self::ALLE.len();
        Self::ALLE[if rueckwaerts {
            (i + n - 1) % n
        } else {
            (i + 1) % n
        }]
    }
}

#[derive(Serialize, Deserialize)]
struct Gespeichert {
    version: u32,
    naechste_id: u64,
    eintraege: Vec<Eintrag>,
}

pub struct Verlauf {
    ordner: PathBuf,
    /// Neueste zuerst.
    pub eintraege: Vec<Eintrag>,
    naechste_id: u64,
}

impl Verlauf {
    /// Lädt den Verlauf aus `ordner`. Eine beschädigte Datei wird beiseitegelegt,
    /// nicht überschrieben, und der Verlauf beginnt leer.
    pub fn laden(ordner: &Path) -> Verlauf {
        let _ = fs::create_dir_all(ordner.join(BILDER));
        let pfad = ordner.join(DATEI);
        let mut v = Verlauf {
            ordner: ordner.to_path_buf(),
            eintraege: Vec::new(),
            naechste_id: 1,
        };
        if let Ok(daten) = fs::read(&pfad) {
            match serde_json::from_slice::<Gespeichert>(&daten) {
                Ok(g) => {
                    v.naechste_id = g
                        .naechste_id
                        .max(g.eintraege.iter().map(|e| e.id + 1).max().unwrap_or(1));
                    v.eintraege = g.eintraege;
                }
                Err(_) => {
                    let _ = fs::rename(&pfad, ordner.join("verlauf.defekt.json"));
                }
            }
        }
        v
    }

    /// Schreibt erst in eine Zwischendatei und ersetzt dann, damit ein Absturz
    /// den Verlauf nicht halb geschrieben zurücklässt.
    pub fn speichern(&self) -> io::Result<()> {
        let g = Gespeichert {
            version: 1,
            naechste_id: self.naechste_id,
            eintraege: self.eintraege.clone(),
        };
        let daten = serde_json::to_vec_pretty(&g).map_err(io::Error::other)?;
        let tmp = self.ordner.join("verlauf.json.tmp");
        fs::write(&tmp, daten)?;
        fs::rename(&tmp, self.ordner.join(DATEI))
    }

    pub fn ordner(&self) -> &Path {
        &self.ordner
    }

    pub fn bild_pfad(&self, e: &Eintrag) -> Option<PathBuf> {
        e.bild.as_ref().map(|n| self.ordner.join(BILDER).join(n))
    }

    pub fn finden(&self, id: u64) -> Option<&Eintrag> {
        self.eintraege.iter().find(|e| e.id == id)
    }

    fn neue_id(&mut self) -> u64 {
        let id = self.naechste_id;
        self.naechste_id += 1;
        id
    }

    fn nach_vorne(&mut self, idx: usize, jetzt: i64, quelle: Option<String>) -> u64 {
        let mut e = self.eintraege.remove(idx);
        e.zeit = jetzt;
        if quelle.is_some() {
            e.quelle = quelle;
        }
        let id = e.id;
        self.eintraege.insert(0, e);
        id
    }

    pub fn text_aufnehmen(
        &mut self,
        text: String,
        quelle: Option<String>,
        jetzt: i64,
    ) -> Option<u64> {
        if text.trim().is_empty() || text.len() > MAX_TEXT_BYTES {
            return None;
        }
        if let Some(i) = self
            .eintraege
            .iter()
            .position(|e| e.art == Art::Text && e.text.as_deref() == Some(text.as_str()))
        {
            return Some(self.nach_vorne(i, jetzt, quelle));
        }
        let id = self.neue_id();
        self.eintraege.insert(
            0,
            Eintrag {
                id,
                art: Art::Text,
                text: Some(text),
                dateien: Vec::new(),
                bild: None,
                breite: 0,
                hoehe: 0,
                pruefsumme: 0,
                quelle,
                zeit: jetzt,
                angeheftet: false,
            },
        );
        self.kuerzen();
        Some(id)
    }

    pub fn dateien_aufnehmen(
        &mut self,
        dateien: Vec<String>,
        quelle: Option<String>,
        jetzt: i64,
    ) -> Option<u64> {
        if dateien.is_empty() {
            return None;
        }
        if let Some(i) = self
            .eintraege
            .iter()
            .position(|e| e.art == Art::Datei && e.dateien == dateien)
        {
            return Some(self.nach_vorne(i, jetzt, quelle));
        }
        let id = self.neue_id();
        self.eintraege.insert(
            0,
            Eintrag {
                id,
                art: Art::Datei,
                text: None,
                dateien,
                bild: None,
                breite: 0,
                hoehe: 0,
                pruefsumme: 0,
                quelle,
                zeit: jetzt,
                angeheftet: false,
            },
        );
        self.kuerzen();
        Some(id)
    }

    /// Nimmt ein Bild als fertig kodierte PNG-Datei auf.
    pub fn bild_aufnehmen(
        &mut self,
        png: &[u8],
        breite: u32,
        hoehe: u32,
        pruefsumme: u64,
        quelle: Option<String>,
        jetzt: i64,
    ) -> io::Result<Option<u64>> {
        if let Some(i) = self.eintraege.iter().position(|e| {
            e.art == Art::Bild
                && e.pruefsumme == pruefsumme
                && e.breite == breite
                && e.hoehe == hoehe
        }) {
            return Ok(Some(self.nach_vorne(i, jetzt, quelle)));
        }
        let id = self.neue_id();
        let name = format!("{id}.png");
        fs::create_dir_all(self.ordner.join(BILDER))?;
        fs::write(self.ordner.join(BILDER).join(&name), png)?;
        self.eintraege.insert(
            0,
            Eintrag {
                id,
                art: Art::Bild,
                text: None,
                dateien: Vec::new(),
                bild: Some(name),
                breite,
                hoehe,
                pruefsumme,
                quelle,
                zeit: jetzt,
                angeheftet: false,
            },
        );
        self.kuerzen();
        Ok(Some(id))
    }

    fn entfernen_an(&mut self, idx: usize) {
        let e = self.eintraege.remove(idx);
        if let Some(p) = self.bild_pfad(&e) {
            let _ = fs::remove_file(p);
        }
    }

    /// Wirft die ältesten nicht angehefteten Einträge hinaus, bis die Grenzen passen.
    pub fn kuerzen(&mut self) {
        loop {
            let bilder = self
                .eintraege
                .iter()
                .filter(|e| !e.angeheftet && e.art == Art::Bild)
                .count();
            if bilder <= MAX_BILDER {
                break;
            }
            match self
                .eintraege
                .iter()
                .rposition(|e| !e.angeheftet && e.art == Art::Bild)
            {
                Some(i) => self.entfernen_an(i),
                None => break,
            }
        }
        loop {
            let frei = self.eintraege.iter().filter(|e| !e.angeheftet).count();
            if frei <= MAX_EINTRAEGE {
                break;
            }
            match self.eintraege.iter().rposition(|e| !e.angeheftet) {
                Some(i) => self.entfernen_an(i),
                None => break,
            }
        }
    }

    pub fn loeschen(&mut self, id: u64) -> bool {
        match self.eintraege.iter().position(|e| e.id == id) {
            Some(i) => {
                self.entfernen_an(i);
                true
            }
            None => false,
        }
    }

    /// Gibt den neuen Zustand zurück.
    pub fn anheften_umschalten(&mut self, id: u64) -> bool {
        match self.eintraege.iter_mut().find(|e| e.id == id) {
            Some(e) => {
                e.angeheftet = !e.angeheftet;
                e.angeheftet
            }
            None => false,
        }
    }

    /// Entfernt nicht angeheftete Texteinträge, auf die `ist_geheim` zutrifft.
    /// Gibt die Anzahl zurück.
    pub fn texte_entfernen(&mut self, ist_geheim: impl Fn(&str) -> bool) -> usize {
        let vorher = self.eintraege.len();
        self.eintraege.retain(|e| {
            e.angeheftet || e.art != Art::Text || !e.text.as_deref().is_some_and(&ist_geheim)
        });
        vorher - self.eintraege.len()
    }

    /// Leert den Verlauf. Angeheftetes bleibt.
    pub fn leeren(&mut self) {
        let mut i = 0;
        while i < self.eintraege.len() {
            if self.eintraege[i].angeheftet {
                i += 1;
            } else {
                self.entfernen_an(i);
            }
        }
    }

    /// Ein Eintrag wurde eingefügt: nach oben holen.
    pub fn benutzt(&mut self, id: u64, jetzt: i64) {
        if let Some(i) = self.eintraege.iter().position(|e| e.id == id) {
            self.nach_vorne(i, jetzt, None);
        }
    }

    /// IDs der Einträge, die im Reiter mit dieser Suche zu sehen sind, in Anzeigereihenfolge.
    pub fn auswahl(&self, reiter: Reiter, suche: &str) -> Vec<u64> {
        let woerter: Vec<String> = suche.split_whitespace().map(|w| w.to_lowercase()).collect();
        self.eintraege
            .iter()
            .filter(|e| match reiter {
                Reiter::Verlauf => true,
                Reiter::Angeheftet => e.angeheftet,
                Reiter::Bilder => e.art == Art::Bild,
            })
            .filter(|e| passt(e, &woerter))
            .map(|e| e.id)
            .collect()
    }
}

/// Jedes Suchwort muss im Text, in einem Dateipfad oder in der Quelle vorkommen.
pub fn passt(e: &Eintrag, woerter: &[String]) -> bool {
    if woerter.is_empty() {
        return true;
    }
    let mut heuhaufen = String::new();
    if let Some(t) = &e.text {
        heuhaufen.push_str(&t.to_lowercase());
        heuhaufen.push('\n');
    }
    for d in &e.dateien {
        heuhaufen.push_str(&d.to_lowercase());
        heuhaufen.push('\n');
    }
    if let Some(q) = &e.quelle {
        heuhaufen.push_str(&q.to_lowercase());
        heuhaufen.push('\n');
    }
    if e.art == Art::Bild {
        heuhaufen.push_str("bild\n");
    }
    woerter.iter().all(|w| heuhaufen.contains(w.as_str()))
}

/// Eine Zeile für die Liste.
pub fn anzeige(e: &Eintrag, nummer: Option<usize>, jetzt: i64) -> String {
    let mut s = String::new();
    match nummer {
        Some(n) => s.push_str(&format!("{n}  ")),
        None => s.push_str("    "),
    }
    if e.angeheftet {
        s.push_str("📌 ");
    }
    let inhalt = match e.art {
        Art::Text => e.text.as_deref().map(erste_zeile).unwrap_or_default(),
        Art::Bild => format!("🖼 Bild {} × {}", e.breite, e.hoehe),
        Art::Datei => {
            let erste = e
                .dateien
                .first()
                .map(|d| d.rsplit(['\\', '/']).next().unwrap_or(d).to_string())
                .unwrap_or_default();
            if e.dateien.len() > 1 {
                format!("📄 {erste} und {} weitere", e.dateien.len() - 1)
            } else {
                format!("📄 {erste}")
            }
        }
    };
    s.push_str(&inhalt);
    s.push_str("   · ");
    s.push_str(&wann(jetzt - e.zeit));
    if let Some(q) = &e.quelle {
        s.push_str(" · ");
        s.push_str(
            q.strip_suffix(".exe")
                .or(q.strip_suffix(".EXE"))
                .unwrap_or(q),
        );
    }
    s
}

/// Erste nichtleere Zeile, auf 90 Zeichen gekürzt.
pub fn erste_zeile(t: &str) -> String {
    let z = t
        .lines()
        .map(str::trim)
        .find(|z| !z.is_empty())
        .unwrap_or("");
    let mut aus: String = z.chars().take(90).collect();
    if z.chars().count() > 90 {
        aus.push('…');
    }
    aus
}

pub fn wann(sekunden: i64) -> String {
    let s = sekunden.max(0);
    match s {
        0..=59 => "gerade eben".into(),
        60..=3599 => format!("vor {} Min.", s / 60),
        3600..=86399 => format!("vor {} Std.", s / 3600),
        86400..=172799 => "gestern".into(),
        _ => format!("vor {} Tagen", s / 86400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ordner(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("hikidashi-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        p
    }

    #[test]
    fn doppelter_text_wandert_nach_oben() {
        let mut v = Verlauf::laden(&ordner("doppelt"));
        let a = v.text_aufnehmen("eins".into(), None, 1).unwrap();
        v.text_aufnehmen("zwei".into(), None, 2).unwrap();
        let a2 = v
            .text_aufnehmen("eins".into(), Some("notepad.exe".into()), 3)
            .unwrap();
        assert_eq!(a, a2);
        assert_eq!(v.eintraege.len(), 2);
        assert_eq!(v.eintraege[0].text.as_deref(), Some("eins"));
        assert_eq!(v.eintraege[0].zeit, 3);
        assert_eq!(v.eintraege[0].quelle.as_deref(), Some("notepad.exe"));
    }

    #[test]
    fn leerer_und_zu_grosser_text_wird_ignoriert() {
        let mut v = Verlauf::laden(&ordner("leer"));
        assert!(v.text_aufnehmen("  \n\t".into(), None, 1).is_none());
        assert!(v
            .text_aufnehmen("x".repeat(MAX_TEXT_BYTES + 1), None, 1)
            .is_none());
        assert!(v.eintraege.is_empty());
    }

    #[test]
    fn kuerzen_laesst_angeheftetes_stehen() {
        let mut v = Verlauf::laden(&ordner("kuerzen"));
        let fest = v.text_aufnehmen("fest".into(), None, 0).unwrap();
        v.anheften_umschalten(fest);
        for i in 0..MAX_EINTRAEGE + 20 {
            v.text_aufnehmen(format!("t{i}"), None, i as i64 + 1);
        }
        assert_eq!(
            v.eintraege.iter().filter(|e| !e.angeheftet).count(),
            MAX_EINTRAEGE
        );
        assert!(v.finden(fest).is_some());
        assert_eq!(
            v.eintraege[0].text.as_deref(),
            Some(&*format!("t{}", MAX_EINTRAEGE + 19))
        );
    }

    #[test]
    fn bildgrenze_loescht_alte_bilddateien() {
        let o = ordner("bilder");
        let mut v = Verlauf::laden(&o);
        let erstes = v.bild_aufnehmen(b"png", 1, 1, 0, None, 0).unwrap().unwrap();
        let pfad = v.bild_pfad(v.finden(erstes).unwrap()).unwrap();
        assert!(pfad.exists());
        for i in 1..=MAX_BILDER as u64 {
            v.bild_aufnehmen(b"png", 1, 1, i, None, i as i64).unwrap();
        }
        assert!(v.finden(erstes).is_none());
        assert!(!pfad.exists());
        assert_eq!(v.eintraege.len(), MAX_BILDER);
    }

    #[test]
    fn leeren_behaelt_angeheftetes_und_speichern_laden_ist_verlustfrei() {
        let o = ordner("speichern");
        let mut v = Verlauf::laden(&o);
        let a = v.text_aufnehmen("Adresse".into(), None, 1).unwrap();
        v.dateien_aufnehmen(vec![r"C:\a.txt".into()], None, 2);
        v.anheften_umschalten(a);
        v.speichern().unwrap();
        let mut w = Verlauf::laden(&o);
        assert_eq!(w.eintraege, v.eintraege);
        w.leeren();
        assert_eq!(w.eintraege.len(), 1);
        assert_eq!(w.eintraege[0].id, a);
        let neu = w.text_aufnehmen("neu".into(), None, 3).unwrap();
        assert!(neu > a, "IDs dürfen sich nach dem Laden nicht wiederholen");
    }

    #[test]
    fn defekte_datei_wird_beiseitegelegt() {
        let o = ordner("defekt");
        fs::create_dir_all(&o).unwrap();
        fs::write(o.join(DATEI), b"{kaputt").unwrap();
        let v = Verlauf::laden(&o);
        assert!(v.eintraege.is_empty());
        assert!(o.join("verlauf.defekt.json").exists());
    }

    #[test]
    fn geheime_texte_entfernen_laesst_angeheftetes_stehen() {
        let mut v = Verlauf::laden(&ordner("geheim"));
        let normal = v.text_aufnehmen("normal".into(), None, 1).unwrap();
        v.text_aufnehmen("GEHEIM-1".into(), None, 2).unwrap();
        let fest = v.text_aufnehmen("GEHEIM-2".into(), None, 3).unwrap();
        v.anheften_umschalten(fest);
        assert_eq!(v.texte_entfernen(|t| t.starts_with("GEHEIM")), 1);
        let ids: Vec<u64> = v.eintraege.iter().map(|e| e.id).collect();
        assert_eq!(ids, vec![fest, normal]);
    }

    #[test]
    fn suche_und_reiter() {
        let mut v = Verlauf::laden(&ordner("suche"));
        let a = v
            .text_aufnehmen("Hallo Welt".into(), Some("WINWORD.EXE".into()), 1)
            .unwrap();
        let b = v.text_aufnehmen("Tschüss".into(), None, 2).unwrap();
        let c = v.bild_aufnehmen(b"png", 2, 2, 7, None, 3).unwrap().unwrap();
        v.anheften_umschalten(b);
        assert_eq!(v.auswahl(Reiter::Verlauf, ""), vec![c, b, a]);
        assert_eq!(v.auswahl(Reiter::Verlauf, "welt HALLO"), vec![a]);
        assert_eq!(v.auswahl(Reiter::Verlauf, "winword"), vec![a]);
        assert_eq!(v.auswahl(Reiter::Angeheftet, ""), vec![b]);
        assert_eq!(v.auswahl(Reiter::Bilder, ""), vec![c]);
        assert!(v.auswahl(Reiter::Verlauf, "hallo tschüss").is_empty());
        v.benutzt(a, 9);
        assert_eq!(v.auswahl(Reiter::Verlauf, "")[0], a);
    }

    #[test]
    fn anzeige_und_zeit() {
        let e = Eintrag {
            id: 1,
            art: Art::Text,
            text: Some("\n  erste Zeile  \nzweite".into()),
            dateien: vec![],
            bild: None,
            breite: 0,
            hoehe: 0,
            pruefsumme: 0,
            quelle: Some("notepad.exe".into()),
            zeit: 0,
            angeheftet: true,
        };
        assert_eq!(
            anzeige(&e, Some(1), 120),
            "1  📌 erste Zeile   · vor 2 Min. · notepad"
        );
        assert_eq!(wann(-5), "gerade eben");
        assert_eq!(wann(7200), "vor 2 Std.");
        assert_eq!(wann(90000), "gestern");
        assert_eq!(wann(86400 * 5), "vor 5 Tagen");
        assert_eq!(erste_zeile(&"a".repeat(100)).chars().count(), 91);
        assert_eq!(Reiter::Bilder.naechster(false), Reiter::Verlauf);
        assert_eq!(Reiter::Verlauf.naechster(true), Reiter::Bilder);
    }
}
