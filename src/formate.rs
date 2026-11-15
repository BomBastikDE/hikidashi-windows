//! Datenformate der Zwischenablage ohne Windows-Aufrufe: DIB-Bilder, Dateilisten
//! (DROPFILES), PNG, Prüfsumme, Passwort-Manager-Liste und das Programmsymbol.

pub struct Rgba {
    pub breite: u32,
    pub hoehe: u32,
    /// Zeilenweise von oben, 4 Bytes je Pixel (R, G, B, A).
    pub pixel: Vec<u8>,
}

fn u16_bei(d: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_le_bytes(d.get(i..i + 2)?.try_into().ok()?))
}
fn u32_bei(d: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes(d.get(i..i + 4)?.try_into().ok()?))
}
fn i32_bei(d: &[u8], i: usize) -> Option<i32> {
    Some(i32::from_le_bytes(d.get(i..i + 4)?.try_into().ok()?))
}

const BI_RGB: u32 = 0;
const BI_BITFIELDS: u32 = 3;
const BI_ALPHABITFIELDS: u32 = 6;

fn kanal(px: u32, maske: u32) -> u8 {
    if maske == 0 {
        return 0;
    }
    let shift = maske.trailing_zeros();
    let max = (maske >> shift) as u64;
    (((px & maske) >> shift) as u64 * 255 / max) as u8
}

/// Liest ein CF_DIB (BITMAPINFOHEADER bzw. V4/V5 plus Pixel). Unterstützt 24 und 32 Bit.
pub fn dib_zu_rgba(d: &[u8], max_bytes: usize) -> Option<Rgba> {
    let kopf = u32_bei(d, 0)? as usize;
    if kopf < 40 {
        return None;
    }
    let breite = i32_bei(d, 4)?;
    let hoehe_roh = i32_bei(d, 8)?;
    let bits = u16_bei(d, 14)?;
    let kompression = u32_bei(d, 16)?;
    let farben = u32_bei(d, 32)? as usize;
    if breite <= 0
        || hoehe_roh == 0
        || hoehe_roh == i32::MIN
        || breite > 30000
        || hoehe_roh.abs() > 30000
    {
        return None;
    }
    let (b, h) = (breite as usize, hoehe_roh.unsigned_abs() as usize);
    if b.checked_mul(h)?.checked_mul(4)? > max_bytes {
        return None;
    }
    let mut versatz = kopf;
    let (mut mr, mut mg, mut mb, mut ma) = (0x00ff_0000u32, 0x0000_ff00u32, 0x0000_00ffu32, 0u32);
    match (bits, kompression) {
        (24, BI_RGB) => {}
        (32, BI_RGB) => ma = 0xff00_0000,
        (32, BI_BITFIELDS) | (32, BI_ALPHABITFIELDS) => {
            if kopf >= 52 {
                mr = u32_bei(d, 40)?;
                mg = u32_bei(d, 44)?;
                mb = u32_bei(d, 48)?;
                ma = if kopf >= 56 { u32_bei(d, 52)? } else { 0 };
            } else {
                mr = u32_bei(d, versatz)?;
                mg = u32_bei(d, versatz + 4)?;
                mb = u32_bei(d, versatz + 8)?;
                versatz += 12;
                if kompression == BI_ALPHABITFIELDS {
                    ma = u32_bei(d, versatz)?;
                    versatz += 4;
                }
            }
        }
        _ => return None,
    }
    versatz += farben.checked_mul(4)?;
    let zeile = (b * bits as usize).div_ceil(32) * 4;
    let ende = versatz.checked_add(zeile.checked_mul(h)?)?;
    let roh = d.get(versatz..ende)?;
    let mut pixel = vec![0u8; b * h * 4];
    let mut alpha_gesehen = false;
    for y in 0..h {
        let quelle_y = if hoehe_roh > 0 { h - 1 - y } else { y };
        let z = &roh[quelle_y * zeile..];
        for x in 0..b {
            let ziel = &mut pixel[(y * b + x) * 4..(y * b + x) * 4 + 4];
            if bits == 24 {
                ziel.copy_from_slice(&[z[x * 3 + 2], z[x * 3 + 1], z[x * 3], 255]);
            } else {
                let px = u32::from_le_bytes(z[x * 4..x * 4 + 4].try_into().ok()?);
                let a = if ma == 0 { 255 } else { kanal(px, ma) };
                alpha_gesehen |= a != 0;
                ziel.copy_from_slice(&[kanal(px, mr), kanal(px, mg), kanal(px, mb), a]);
            }
        }
    }
    // Viele Programme lassen den Alphakanal bei 32 Bit leer (alles 0): dann deckend.
    if bits == 32 && !alpha_gesehen {
        for p in pixel.chunks_exact_mut(4) {
            p[3] = 255;
        }
    }
    Some(Rgba {
        breite: b as u32,
        hoehe: h as u32,
        pixel,
    })
}

/// Baut ein CF_DIB mit 32 Bit, von unten nach oben, BGRA.
pub fn rgba_zu_dib(bild: &Rgba) -> Vec<u8> {
    let (b, h) = (bild.breite as usize, bild.hoehe as usize);
    let mut d = Vec::with_capacity(40 + b * h * 4);
    d.extend_from_slice(&40u32.to_le_bytes());
    d.extend_from_slice(&(b as i32).to_le_bytes());
    d.extend_from_slice(&(h as i32).to_le_bytes());
    d.extend_from_slice(&1u16.to_le_bytes());
    d.extend_from_slice(&32u16.to_le_bytes());
    d.extend_from_slice(&BI_RGB.to_le_bytes());
    d.extend_from_slice(&((b * h * 4) as u32).to_le_bytes());
    d.extend_from_slice(&[0u8; 16]);
    for y in (0..h).rev() {
        for p in bild.pixel[y * b * 4..(y + 1) * b * 4].chunks_exact(4) {
            d.extend_from_slice(&[p[2], p[1], p[0], p[3]]);
        }
    }
    d
}

pub fn png_kodieren(bild: &Rgba) -> Result<Vec<u8>, String> {
    let mut aus = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut aus, bild.breite, bild.hoehe);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_compression(png::Compression::Fast);
        let mut w = enc.write_header().map_err(|e| e.to_string())?;
        w.write_image_data(&bild.pixel).map_err(|e| e.to_string())?;
    }
    Ok(aus)
}

pub fn png_dekodieren(daten: &[u8]) -> Result<Rgba, String> {
    let mut dec = png::Decoder::new(std::io::Cursor::new(daten));
    dec.set_transformations(png::Transformations::normalize_to_color8());
    let mut r = dec.read_info().map_err(|e| e.to_string())?;
    let groesse = r.output_buffer_size().ok_or("Bild zu groß")?;
    let mut puffer = vec![0u8; groesse];
    let info = r.next_frame(&mut puffer).map_err(|e| e.to_string())?;
    let n = (info.width * info.height) as usize;
    let roh = &puffer[..info.buffer_size()];
    let pixel = match info.color_type {
        png::ColorType::Rgba => roh.to_vec(),
        png::ColorType::Rgb => roh
            .chunks_exact(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => roh
            .chunks_exact(2)
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        png::ColorType::Grayscale => roh.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return Err("Palettenbild nicht erwartet".into()),
    };
    if pixel.len() != n * 4 {
        return Err("Bildgröße passt nicht".into());
    }
    Ok(Rgba {
        breite: info.width,
        hoehe: info.height,
        pixel,
    })
}

/// FNV-1a, 64 Bit. Reicht, um gleiche Bilder wiederzuerkennen.
pub fn pruefsumme(daten: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in daten {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// Liest eine DROPFILES-Struktur (CF_HDROP).
pub fn dropfiles_lesen(d: &[u8]) -> Vec<String> {
    let (Some(start), Some(breit)) = (u32_bei(d, 0), u32_bei(d, 16)) else {
        return Vec::new();
    };
    let rest = d.get(start as usize..).unwrap_or(&[]);
    let teile: Vec<String> = if breit != 0 {
        let einheiten: Vec<u16> = rest
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        einheiten
            .split(|&c| c == 0)
            .take_while(|s| !s.is_empty())
            .map(String::from_utf16_lossy)
            .collect()
    } else {
        rest.split(|&c| c == 0)
            .take_while(|s| !s.is_empty())
            .map(|s| s.iter().map(|&b| b as char).collect())
            .collect()
    };
    teile
}

/// Baut eine DROPFILES-Struktur mit UTF-16-Pfaden.
pub fn dropfiles_bauen(pfade: &[String]) -> Vec<u8> {
    let mut d = Vec::new();
    d.extend_from_slice(&20u32.to_le_bytes()); // pFiles
    d.extend_from_slice(&[0u8; 8]); // pt
    d.extend_from_slice(&0u32.to_le_bytes()); // fNC
    d.extend_from_slice(&1u32.to_le_bytes()); // fWide
    for p in pfade {
        for c in p.encode_utf16().chain(std::iter::once(0)) {
            d.extend_from_slice(&c.to_le_bytes());
        }
    }
    d.extend_from_slice(&0u16.to_le_bytes());
    d
}

/// Programme, aus denen nie etwas aufgenommen wird.
const PASSWORT_MANAGER: &[&str] = &[
    "1password.exe",
    "bitwarden.exe",
    "keepass.exe",
    "keepassxc.exe",
    "dashlane.exe",
    "lastpass.exe",
    "enpass.exe",
    "proton pass.exe",
    "nordpass.exe",
    "roboform.exe",
    "keeper password manager.exe",
    "credentialuibroker.exe",
];

pub fn ist_passwortmanager(exe: &str) -> bool {
    let name = exe.rsplit(['\\', '/']).next().unwrap_or(exe).to_lowercase();
    PASSWORT_MANAGER.contains(&name.as_str())
}

/// Erkennt Text, der wie ein Passwort aussieht: ein einzelnes Wort mit 8 bis 64 Zeichen
/// aus mindestens drei Zeichenarten (Klein, Groß, Ziffer, Sonderzeichen). Adressen,
/// E-Mail, Pfade, UUIDs und ISO-Daten zählen nicht. Gedacht für Passwörter aus
/// Browser-Erweiterungen, die keine Geheim-Markierung setzen (Issue #3).
pub fn sieht_aus_wie_passwort(t: &str) -> bool {
    let n = t.chars().count();
    if !(8..=64).contains(&n) || t.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return false;
    }
    let klein = t.chars().any(|c| c.is_lowercase());
    let gross = t.chars().any(|c| c.is_uppercase());
    let ziffer = t.chars().any(|c| c.is_ascii_digit());
    let sonder = t.chars().any(|c| !c.is_alphanumeric());
    if [klein, gross, ziffer, sonder]
        .iter()
        .filter(|&&b| b)
        .count()
        < 3
    {
        return false;
    }
    !(ist_adresse(t) || ist_email(t) || ist_pfad(t) || ist_uuid(t) || ist_iso_datum(t))
}

fn ist_adresse(t: &str) -> bool {
    let l = t.to_lowercase();
    l.contains("://") || l.starts_with("www.") || l.starts_with("mailto:")
}

fn ist_email(t: &str) -> bool {
    let Some((lokal, domain)) = t.split_once('@') else {
        return false;
    };
    let Some((name, tld)) = domain.rsplit_once('.') else {
        return false;
    };
    !lokal.is_empty()
        && !name.is_empty()
        && lokal
            .chars()
            .all(|c| c.is_alphanumeric() || "._%+-".contains(c))
        && domain
            .chars()
            .all(|c| c.is_alphanumeric() || ".-".contains(c))
        && tld.chars().count() >= 2
        && tld.chars().all(|c| c.is_alphabetic())
}

fn ist_pfad(t: &str) -> bool {
    let b = t.as_bytes();
    t.starts_with("\\\\")
        || t.starts_with('/')
        || t.starts_with("~/")
        || t.starts_with(".\\")
        || t.starts_with("./")
        || (b.len() > 2
            && b[0].is_ascii_alphabetic()
            && b[1] == b':'
            && (b[2] == b'\\' || b[2] == b'/'))
}

fn ist_uuid(t: &str) -> bool {
    let teile: Vec<&str> = t.trim_matches(['{', '}']).split('-').collect();
    teile.iter().map(|s| s.len()).eq([8, 4, 4, 4, 12])
        && teile
            .iter()
            .all(|s| s.chars().all(|c| c.is_ascii_hexdigit()))
}

fn ist_iso_datum(t: &str) -> bool {
    let b = t.as_bytes();
    b.len() >= 10
        && b[..4].iter().all(u8::is_ascii_digit)
        && b[4] == b'-'
        && b[5..7].iter().all(u8::is_ascii_digit)
        && b[7] == b'-'
        && b[8..10].iter().all(u8::is_ascii_digit)
}

/// Symbol einer Schublade, BGRA von oben nach unten, für CreateIcon.
pub fn symbol_bgra(g: u32) -> Vec<u8> {
    let s = g as f32 / 32.0;
    let mut d = vec![0u8; (g * g * 4) as usize];
    let gerundet = |x: f32, y: f32, x0: f32, y0: f32, x1: f32, y1: f32, r: f32| -> bool {
        if x < x0 || x > x1 || y < y0 || y > y1 {
            return false;
        }
        let cx = x.clamp(x0 + r, x1 - r);
        let cy = y.clamp(y0 + r, y1 - r);
        (x - cx).powi(2) + (y - cy).powi(2) <= r * r
    };
    for y in 0..g {
        for x in 0..g {
            let (fx, fy) = ((x as f32 + 0.5) / s, (y as f32 + 0.5) / s);
            let i = ((y * g + x) * 4) as usize;
            // Korpus: warmes Orange, eine dunklere Fuge, zwei helle Griffe.
            let farbe: Option<[u8; 3]> = if !gerundet(fx, fy, 2.0, 5.0, 30.0, 28.0, 4.0) {
                None
            } else if (15.5..17.0).contains(&fy) {
                Some([0x9a, 0x4a, 0x16])
            } else if gerundet(fx, fy, 12.0, 9.5, 20.0, 12.5, 1.5)
                || gerundet(fx, fy, 12.0, 20.5, 20.0, 23.5, 1.5)
            {
                Some([0xff, 0xf3, 0xe6])
            } else {
                Some([0xe8, 0x83, 0x3a])
            };
            if let Some([r, gr, b]) = farbe {
                d[i..i + 4].copy_from_slice(&[b, gr, r, 255]);
            }
        }
    }
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    fn testbild() -> Rgba {
        Rgba {
            breite: 3,
            hoehe: 2,
            pixel: vec![
                255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, //
                10, 20, 30, 128, 40, 50, 60, 0, 70, 80, 90, 255,
            ],
        }
    }

    #[test]
    fn dib_hin_und_zurueck() {
        let b = testbild();
        let dib = rgba_zu_dib(&b);
        let r = dib_zu_rgba(&dib, 1 << 20).unwrap();
        assert_eq!((r.breite, r.hoehe), (3, 2));
        assert_eq!(r.pixel, b.pixel);
    }

    #[test]
    fn dib_24_bit_von_oben_mit_zeilenauffuellung() {
        // 1 × 2 Pixel, Höhe negativ (von oben), Zeile auf 4 Bytes aufgefüllt.
        let mut d = Vec::new();
        d.extend_from_slice(&40u32.to_le_bytes());
        d.extend_from_slice(&1i32.to_le_bytes());
        d.extend_from_slice(&(-2i32).to_le_bytes());
        d.extend_from_slice(&1u16.to_le_bytes());
        d.extend_from_slice(&24u16.to_le_bytes());
        d.extend_from_slice(&[0u8; 24]);
        d.extend_from_slice(&[3, 2, 1, 0, 6, 5, 4, 0]);
        let r = dib_zu_rgba(&d, 1 << 20).unwrap();
        assert_eq!(r.pixel, vec![1, 2, 3, 255, 4, 5, 6, 255]);
    }

    #[test]
    fn dib_32_bit_ohne_alpha_wird_deckend() {
        let mut b = testbild();
        for p in b.pixel.chunks_exact_mut(4) {
            p[3] = 0;
        }
        let r = dib_zu_rgba(&rgba_zu_dib(&b), 1 << 20).unwrap();
        assert!(r.pixel.chunks_exact(4).all(|p| p[3] == 255));
    }

    #[test]
    fn dib_bitfields_und_unsinn() {
        let mut d = Vec::new();
        d.extend_from_slice(&40u32.to_le_bytes());
        d.extend_from_slice(&1i32.to_le_bytes());
        d.extend_from_slice(&1i32.to_le_bytes());
        d.extend_from_slice(&1u16.to_le_bytes());
        d.extend_from_slice(&32u16.to_le_bytes());
        d.extend_from_slice(&3u32.to_le_bytes());
        d.extend_from_slice(&[0u8; 20]);
        for m in [0x0000_00ffu32, 0x0000_ff00, 0x00ff_0000] {
            d.extend_from_slice(&m.to_le_bytes());
        }
        d.extend_from_slice(&[9, 8, 7, 0]);
        assert_eq!(dib_zu_rgba(&d, 1 << 20).unwrap().pixel, vec![9, 8, 7, 255]);
        assert!(dib_zu_rgba(&d[..50], 1 << 20).is_none(), "abgeschnitten");
        assert!(dib_zu_rgba(&d, 3).is_none(), "über der Grenze");
        assert!(dib_zu_rgba(&[0u8; 8], 1 << 20).is_none());
    }

    #[test]
    fn png_hin_und_zurueck() {
        let b = testbild();
        let r = png_dekodieren(&png_kodieren(&b).unwrap()).unwrap();
        assert_eq!(r.pixel, b.pixel);
        assert!(png_dekodieren(b"kein png").is_err());
    }

    #[test]
    fn dropfiles_hin_und_zurueck() {
        let pfade = vec![
            r"C:\Users\ä\Bericht.pdf".to_string(),
            r"D:\x y\🍣.txt".to_string(),
        ];
        assert_eq!(dropfiles_lesen(&dropfiles_bauen(&pfade)), pfade);
        assert!(dropfiles_lesen(&[1, 2]).is_empty());
    }

    #[test]
    fn passwortaehnliches_wird_erkannt() {
        for pw in [
            "Xk9#mP2$vL",                 // generiert, vier Zeichenarten
            "Sommer2024!",                // menschlich
            "correct-Horse-battery",      // klein, groß, Sonderzeichen
            "aB3dE6gH9jK2mN5pQ8sT1vW4yZ", // ohne Sonderzeichen, drei Arten
            "p@ssW0rd",                   // @ im Passwort, keine E-Mail
            "Ünïcode-Pässwört1",
        ] {
            assert!(sieht_aus_wie_passwort(pw), "{pw}");
        }
        for kein in [
            "kurz1A!",                              // 7 Zeichen
            "nurkleinbuchstaben",                   // eine Zeichenart
            "abc123def456",                         // zwei Zeichenarten
            "Hallo Welt 2024!",                     // Leerzeichen
            "https://Beispiel.de/A1",               // Adresse
            "www.Beispiel-1.de",                    // Adresse
            "Max.Mustermann@Beispiel.de",           // E-Mail
            r"C:\Users\Max\Bericht_2026.pdf",       // Pfad
            r"\\server\Freigabe\Datei1",            // UNC-Pfad
            "/home/Max/Datei_1",                    // Unix-Pfad
            "550e8400-e29b-41d4-A716-446655440000", // UUID
            "2026-11-13T18:46:00Z",                 // ISO-Datum
            "Ab1!".repeat(17).as_str(),             // 68 Zeichen
        ] {
            assert!(!sieht_aus_wie_passwort(kein), "{kein}");
        }
    }

    #[test]
    fn passwortmanager_und_pruefsumme() {
        assert!(ist_passwortmanager(
            r"C:\Program Files\KeePassXC\KeePassXC.exe"
        ));
        assert!(ist_passwortmanager("1Password.exe"));
        assert!(!ist_passwortmanager("notepad.exe"));
        assert_ne!(pruefsumme(b"a"), pruefsumme(b"b"));
        assert_eq!(symbol_bgra(16).len(), 16 * 16 * 4);
    }

    /// BITMAPINFOHEADER-Grundkopf mit frei wählbarer Kopflänge.
    fn dib_kopf(kopf: u32, breite: i32, hoehe: i32, bits: u16, kompression: u32) -> Vec<u8> {
        let mut d = Vec::new();
        d.extend_from_slice(&kopf.to_le_bytes());
        d.extend_from_slice(&breite.to_le_bytes());
        d.extend_from_slice(&hoehe.to_le_bytes());
        d.extend_from_slice(&1u16.to_le_bytes());
        d.extend_from_slice(&bits.to_le_bytes());
        d.extend_from_slice(&kompression.to_le_bytes());
        d.extend_from_slice(&[0u8; 20]);
        d
    }

    #[test]
    fn dib_v5_kopf_liest_masken_samt_alpha_aus_dem_kopf() {
        // V5-Kopf (124 Bytes): Masken bei 40..56, Rest des Kopfes leer.
        let mut d = dib_kopf(124, 1, 1, 32, BI_BITFIELDS);
        for m in [0x00ff_0000u32, 0x0000_ff00, 0x0000_00ff, 0xff00_0000] {
            d.extend_from_slice(&m.to_le_bytes());
        }
        d.resize(124, 0);
        d.extend_from_slice(&[30, 20, 10, 128]); // B, G, R, A
        assert_eq!(
            dib_zu_rgba(&d, 1 << 20).unwrap().pixel,
            vec![10, 20, 30, 128]
        );
    }

    #[test]
    fn dib_alphabitfields_liest_die_alphamaske_hinter_dem_kopf() {
        let mut d = dib_kopf(40, 1, 1, 32, BI_ALPHABITFIELDS);
        for m in [0x00ff_0000u32, 0x0000_ff00, 0x0000_00ff, 0xff00_0000] {
            d.extend_from_slice(&m.to_le_bytes());
        }
        d.extend_from_slice(&[30, 20, 10, 64]);
        assert_eq!(
            dib_zu_rgba(&d, 1 << 20).unwrap().pixel,
            vec![10, 20, 30, 64]
        );
    }

    #[test]
    fn dib_ueberspringt_die_farbtabelle() {
        let mut d = dib_kopf(40, 1, 1, 24, BI_RGB);
        d[32..36].copy_from_slice(&2u32.to_le_bytes()); // biClrUsed = 2
        d.extend_from_slice(&[0xee; 8]); // zwei Tabelleneinträge
        d.extend_from_slice(&[3, 2, 1, 0]);
        assert_eq!(dib_zu_rgba(&d, 1 << 20).unwrap().pixel, vec![1, 2, 3, 255]);
    }

    #[test]
    fn dib_24_bit_von_unten_wird_umgedreht() {
        let mut d = dib_kopf(40, 1, 2, 24, BI_RGB);
        d.extend_from_slice(&[3, 2, 1, 0, 6, 5, 4, 0]); // untere Zeile zuerst
        assert_eq!(
            dib_zu_rgba(&d, 1 << 20).unwrap().pixel,
            vec![4, 5, 6, 255, 1, 2, 3, 255]
        );
    }

    #[test]
    fn dib_lehnt_unsinnige_masse_und_formate_ab() {
        let pixel = [0u8; 64];
        for (breite, hoehe, bits, kompression, warum) in [
            (0, 1, 32, BI_RGB, "Breite 0"),
            (-1, 1, 32, BI_RGB, "negative Breite"),
            (1, 0, 32, BI_RGB, "Höhe 0"),
            (1, i32::MIN, 32, BI_RGB, "Höhe i32::MIN"),
            (30001, 1, 32, BI_RGB, "zu breit"),
            (1, -30001, 32, BI_RGB, "zu hoch"),
            (1, 1, 8, BI_RGB, "8 Bit mit Palette"),
            (1, 1, 16, BI_RGB, "16 Bit"),
            (1, 1, 24, 1, "RLE8"),
        ] {
            let mut d = dib_kopf(40, breite, hoehe, bits, kompression);
            d.extend_from_slice(&pixel);
            assert!(dib_zu_rgba(&d, 1 << 30).is_none(), "{warum}");
        }
        let mut kurzer_kopf = dib_kopf(12, 1, 1, 24, BI_RGB);
        kurzer_kopf.extend_from_slice(&pixel);
        assert!(dib_zu_rgba(&kurzer_kopf, 1 << 20).is_none(), "OS/2-Kopf");
    }

    #[test]
    fn dib_mit_riesiger_farbtabelle_laeuft_nicht_ueber() {
        let mut d = dib_kopf(40, 1, 1, 24, BI_RGB);
        d[32..36].copy_from_slice(&u32::MAX.to_le_bytes());
        d.extend_from_slice(&[0u8; 4]);
        assert!(dib_zu_rgba(&d, 1 << 20).is_none());
    }

    #[test]
    fn kanal_skaliert_schmale_masken_auf_8_bit() {
        assert_eq!(kanal(0b11111, 0b11111), 255); // 5 Bit voll
        assert_eq!(kanal(0, 0b11111), 0);
        assert_eq!(kanal(0xffff_ffff, 0), 0, "fehlende Maske");
    }

    fn png_mit(farbe: png::ColorType, breite: u32, hoehe: u32, daten: &[u8]) -> Vec<u8> {
        let mut aus = Vec::new();
        let mut enc = png::Encoder::new(&mut aus, breite, hoehe);
        enc.set_color(farbe);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header().unwrap().write_image_data(daten).unwrap();
        aus
    }

    #[test]
    fn png_rgb_wird_deckend() {
        let p = png_mit(png::ColorType::Rgb, 1, 1, &[1, 2, 3]);
        assert_eq!(png_dekodieren(&p).unwrap().pixel, vec![1, 2, 3, 255]);
    }

    #[test]
    fn png_graustufen_mit_und_ohne_alpha() {
        let g = png_mit(png::ColorType::Grayscale, 2, 1, &[7, 9]);
        assert_eq!(
            png_dekodieren(&g).unwrap().pixel,
            vec![7, 7, 7, 255, 9, 9, 9, 255]
        );
        let ga = png_mit(png::ColorType::GrayscaleAlpha, 1, 1, &[7, 100]);
        assert_eq!(png_dekodieren(&ga).unwrap().pixel, vec![7, 7, 7, 100]);
    }

    #[test]
    fn png_mit_palette_wird_zu_rgba() {
        let mut aus = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut aus, 2, 1);
            enc.set_color(png::ColorType::Indexed);
            enc.set_depth(png::BitDepth::Eight);
            enc.set_palette(vec![10, 20, 30, 40, 50, 60]);
            enc.write_header()
                .unwrap()
                .write_image_data(&[1, 0])
                .unwrap();
        }
        let r = png_dekodieren(&aus).unwrap();
        assert_eq!(r.pixel, vec![40, 50, 60, 255, 10, 20, 30, 255]);
    }

    #[test]
    fn dropfiles_ansi_wird_gelesen() {
        let mut d = Vec::new();
        d.extend_from_slice(&20u32.to_le_bytes());
        d.extend_from_slice(&[0u8; 12]); // pt, fNC
        d.extend_from_slice(&0u32.to_le_bytes()); // fWide = 0
        d.extend_from_slice(b"C:\\a.txt\0D:\\b.txt\0\0");
        assert_eq!(dropfiles_lesen(&d), vec![r"C:\a.txt", r"D:\b.txt"]);
    }

    #[test]
    fn dropfiles_mit_versatz_hinter_dem_ende_ist_leer() {
        let mut d = dropfiles_bauen(&["x".into()]);
        d[0..4].copy_from_slice(&9999u32.to_le_bytes());
        assert!(dropfiles_lesen(&d).is_empty());
    }

    #[test]
    fn dropfiles_ohne_pfade_ist_leer() {
        assert!(dropfiles_lesen(&dropfiles_bauen(&[])).is_empty());
    }

    #[test]
    fn passwortmanager_auch_mit_schraegstrich_und_leerzeichen_im_namen() {
        assert!(ist_passwortmanager(
            "C:/Programme/Proton Pass/Proton Pass.exe"
        ));
        assert!(!ist_passwortmanager("keepass.exe.bak"));
        assert!(!ist_passwortmanager(""));
    }

    #[test]
    fn passwortgrenzen_8_und_64_zeichen() {
        assert!(sieht_aus_wie_passwort("Ab1!Ab1!"));
        assert!(sieht_aus_wie_passwort(&"Ab1!".repeat(16)));
        assert!(!sieht_aus_wie_passwort("Ab1!Ab1"));
        assert!(!sieht_aus_wie_passwort("Ab1!Ab1!\u{7}"), "Steuerzeichen");
    }

    #[test]
    fn email_ohne_gueltige_endung_kann_passwort_sein() {
        // Kein E-Mail-Muster: Endung mit Ziffer.
        assert!(sieht_aus_wie_passwort("Abc@def.g1"));
        assert!(!sieht_aus_wie_passwort("Abc1@def.gh"), "echte E-Mail-Form");
    }

    #[test]
    fn uuid_in_geschweiften_klammern_und_mailto_sind_kein_passwort() {
        assert!(!sieht_aus_wie_passwort(
            "{550E8400-e29b-41d4-a716-446655440000}"
        ));
        assert!(!sieht_aus_wie_passwort("mailto:Max1@x.de"));
        assert!(!sieht_aus_wie_passwort(r".\Ordner\Datei1"));
    }

    #[test]
    fn pruefsumme_ist_fnv1a() {
        assert_eq!(pruefsumme(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(pruefsumme(b"a"), 0xaf63_dc4c_8601_ec8c);
    }

    #[test]
    fn symbol_hat_durchsichtige_ecken_und_deckende_mitte() {
        let g = 32;
        let d = symbol_bgra(g);
        let alpha = |x: u32, y: u32| d[((y * g + x) * 4 + 3) as usize];
        assert_eq!(alpha(0, 0), 0);
        assert_eq!(alpha(16, 20), 255);
    }
}
