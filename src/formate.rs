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
}
