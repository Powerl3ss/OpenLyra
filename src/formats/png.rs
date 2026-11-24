//! PNG : analyse des blocs (chunks) de texte, EXIF, XMP, date…

use crate::exifmeta;
use crate::model::{Entry, Group, Risk};
use crate::xmp;
use anyhow::{bail, Result};
use std::collections::HashSet;
use std::io::Read;

const SIG: &[u8] = b"\x89PNG\r\n\x1a\n";

/// Blocs auxiliaires nécessaires à l'affichage, toujours conservés.
const SAFE: &[&[u8; 4]] = &[
    b"tRNS", b"gAMA", b"cHRM", b"sRGB", b"sBIT", b"bKGD", b"hIST", b"pHYs", b"sPLT", b"acTL", b"fcTL", b"fdAT",
    b"cICP", b"mDCv", b"cLLI",
];

struct Chunk<'a> {
    kind: [u8; 4],
    data: &'a [u8],
    raw: &'a [u8],
}

fn chunks(b: &[u8]) -> Result<Vec<Chunk<'_>>> {
    if !b.starts_with(SIG) {
        bail!("Fichier PNG invalide");
    }
    let mut i = SIG.len();
    let mut v = Vec::new();
    while i + 12 <= b.len() {
        let len = u32::from_be_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]) as usize;
        let end = i + 12 + len;
        if end > b.len() {
            bail!("Bloc PNG tronqué");
        }
        let kind = [b[i + 4], b[i + 5], b[i + 6], b[i + 7]];
        v.push(Chunk { kind, data: &b[i + 8..i + 8 + len], raw: &b[i..end] });
        i = end;
        if &kind == b"IEND" {
            break;
        }
    }
    Ok(v)
}

#[derive(PartialEq)]
enum Kind {
    Text,
    Xmp,
    Exif,
    Time,
    Icc,
    Other,
    Keep,
}

fn classify(c: &Chunk) -> Kind {
    match &c.kind {
        b"tEXt" | b"zTXt" => Kind::Text,
        b"iTXt" => {
            if c.data.starts_with(b"XML:com.adobe.xmp\0") {
                Kind::Xmp
            } else {
                Kind::Text
            }
        }
        b"eXIf" => Kind::Exif,
        b"tIME" => Kind::Time,
        b"iCCP" => Kind::Icc,
        k if k[0].is_ascii_uppercase() => Kind::Keep, // bloc critique
        k if SAFE.iter().any(|s| *s == k) => Kind::Keep,
        _ => Kind::Other,
    }
}

fn inflate(d: &[u8]) -> String {
    let mut s = String::new();
    let mut dec = flate2::read::ZlibDecoder::new(d);
    if dec.read_to_string(&mut s).is_err() {
        return format!("Texte compressé ({} octets)", d.len());
    }
    s
}

fn split_nul(d: &[u8]) -> (&[u8], &[u8]) {
    match d.iter().position(|&c| c == 0) {
        Some(p) => (&d[..p], &d[p + 1..]),
        None => (d, &[]),
    }
}

fn latin1(d: &[u8]) -> String {
    d.iter().map(|&c| c as char).collect()
}

fn text_entry(c: &Chunk) -> Entry {
    let (key, rest) = split_nul(c.data);
    let key = latin1(key);
    let value = match &c.kind {
        b"tEXt" => latin1(rest),
        b"zTXt" => inflate(rest.get(1..).unwrap_or(&[])),
        _ => {
            // iTXt : drapeau compression, méthode, langue\0, mot-clé traduit\0, texte
            let compressed = rest.first().copied().unwrap_or(0) == 1;
            let rest = rest.get(2..).unwrap_or(&[]);
            let (_, rest) = split_nul(rest);
            let (_, text) = split_nul(rest);
            if compressed {
                inflate(text)
            } else {
                String::from_utf8_lossy(text).into_owned()
            }
        }
    };
    let label = match key.as_str() {
        "Author" => "Auteur (Author)".to_string(),
        "Software" => "Logiciel (Software)".to_string(),
        "Creation Time" => "Date de création".to_string(),
        "Comment" => "Commentaire".to_string(),
        "Description" => "Description".to_string(),
        "Copyright" => "Droits d'auteur".to_string(),
        "Title" => "Titre".to_string(),
        _ => key,
    };
    Entry::new(label, value)
}

pub fn analyze(b: &[u8]) -> Result<(Vec<Group>, Vec<String>)> {
    let list = chunks(b)?;
    let mut groups = Vec::new();
    let (mut text, mut xmp_e, mut time, mut other) = (vec![], vec![], vec![], vec![]);
    let mut icc = 0;
    for c in &list {
        match classify(c) {
            Kind::Text => text.push(text_entry(c)),
            Kind::Xmp => {
                let (_, rest) = split_nul(c.data);
                let rest = rest.get(2..).unwrap_or(&[]);
                let (_, rest) = split_nul(rest);
                let (_, packet) = split_nul(rest);
                xmp_e.extend(xmp::entries(packet));
            }
            Kind::Exif => groups.extend(exifmeta::analyze(c.data)),
            Kind::Time => {
                let d = c.data;
                if d.len() >= 7 {
                    let y = u16::from_be_bytes([d[0], d[1]]);
                    time.push(Entry::new(
                        "Dernière modification",
                        format!("{:02}/{:02}/{} à {:02}:{:02}:{:02}", d[3], d[2], y, d[4], d[5], d[6]),
                    ));
                }
            }
            Kind::Icc => icc += c.data.len(),
            Kind::Other => other.push(Entry::new(
                format!("Bloc {}", String::from_utf8_lossy(&c.kind)),
                format!("{} octets", c.data.len()),
            )),
            Kind::Keep => {}
        }
    }
    if !text.is_empty() {
        let risk = if text.iter().any(|e| e.key.contains("Auteur") || e.key.contains("Commentaire")) {
            Risk::High
        } else {
            Risk::Medium
        };
        groups.push(
            Group::new(
                "png.text",
                "Textes intégrés",
                "Champs texte (auteur, logiciel, commentaire, paramètres de génération…).",
                risk,
            )
            .entries(text),
        );
    }
    if !xmp_e.is_empty() {
        let r = xmp::risk_of(&xmp_e);
        groups.push(
            Group::new("xmp", "Métadonnées XMP", "Bloc Adobe : auteur, logiciels, historique.", r).entries(xmp_e),
        );
    }
    if !time.is_empty() {
        groups.push(
            Group::new("png.time", "Date de modification", "Horodatage enregistré par le logiciel.", Risk::Medium)
                .entries(time),
        );
    }
    if !other.is_empty() {
        groups.push(
            Group::new(
                "png.other",
                "Blocs non standard",
                "Données ajoutées par un logiciel, inutiles à l'affichage.",
                Risk::Medium,
            )
            .entries(other),
        );
    }
    if icc > 0 {
        groups.push(
            Group::new("icc", "Profil de couleur ICC", "Décrit comment afficher les couleurs.", Risk::Low)
                .not_recommended()
                .note("Le supprimer peut légèrement modifier le rendu des couleurs.")
                .entries(vec![Entry::new("Taille", format!("{} octets", icc))]),
        );
    }
    Ok((groups, vec![]))
}

fn write_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc = flate2::Crc::new();
    crc.update(kind);
    crc.update(data);
    out.extend_from_slice(&crc.sum().to_be_bytes());
}

pub fn clean(b: &[u8], sel: &HashSet<String>) -> Result<(Vec<u8>, Vec<String>)> {
    let list = chunks(b)?;
    let mut out = Vec::with_capacity(b.len());
    let mut notes = Vec::new();
    out.extend_from_slice(SIG);
    let any_exif = sel.iter().any(|s| s.starts_with("exif."));
    for c in &list {
        let keep = match classify(c) {
            Kind::Text => !sel.contains("png.text"),
            Kind::Xmp => !sel.contains("xmp"),
            Kind::Time => !sel.contains("png.time"),
            Kind::Icc => !sel.contains("icc"),
            Kind::Other => !sel.contains("png.other"),
            Kind::Keep => true,
            Kind::Exif => {
                if any_exif {
                    match exifmeta::rebuild(c.data, &|g| sel.contains(g)) {
                        Ok(Some(t)) => write_chunk(&mut out, b"eXIf", &t),
                        Ok(None) => {}
                        Err(e) => notes.push(format!("Bloc EXIF supprimé en entier ({}).", e)),
                    }
                    false
                } else {
                    true
                }
            }
        };
        if keep {
            out.extend_from_slice(c.raw);
        }
    }
    Ok((out, notes))
}
