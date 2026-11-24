//! JPEG : lecture des segments APPn / COM et réécriture sans copie de l'image.

use crate::exifmeta;
use crate::model::{Entry, Group, Risk};
use crate::xmp;
use anyhow::{bail, Result};
use std::collections::HashSet;

const EXIF_SIG: &[u8] = b"Exif\0\0";
const XMP_SIG: &[u8] = b"http://ns.adobe.com/xap/1.0/\0";
const XMP_EXT_SIG: &[u8] = b"http://ns.adobe.com/xmp/extension/\0";
const PS_SIG: &[u8] = b"Photoshop 3.0\0";
const ICC_SIG: &[u8] = b"ICC_PROFILE\0";

#[derive(Debug, PartialEq)]
enum Kind {
    Exif,
    Xmp,
    XmpExt,
    Iptc,
    Icc,
    Comment,
    OtherApp,
    Keep,
}

struct Segment {
    marker: u8,
    start: usize,
    end: usize,
    data_start: usize,
}

struct Parsed {
    segments: Vec<Segment>,
    scan_start: usize,
    eoi_end: Option<usize>,
}

fn parse(b: &[u8]) -> Result<Parsed> {
    if b.len() < 4 || b[0] != 0xFF || b[1] != 0xD8 {
        bail!("Fichier JPEG invalide");
    }
    let mut i = 2;
    let mut segments = Vec::new();
    loop {
        if i + 1 >= b.len() {
            bail!("Fichier JPEG tronqué");
        }
        if b[i] != 0xFF {
            bail!("Structure JPEG inattendue à l'octet {}", i);
        }
        while i + 1 < b.len() && b[i + 1] == 0xFF {
            i += 1;
        }
        let marker = b[i + 1];
        if marker == 0xDA || marker == 0xD9 {
            break;
        }
        if (0xD0..=0xD7).contains(&marker) || marker == 0x01 {
            segments.push(Segment { marker, start: i, end: i + 2, data_start: i + 2 });
            i += 2;
            continue;
        }
        if i + 4 > b.len() {
            bail!("Fichier JPEG tronqué");
        }
        let len = u16::from_be_bytes([b[i + 2], b[i + 3]]) as usize;
        let end = i + 2 + len;
        if len < 2 || end > b.len() {
            bail!("Segment JPEG corrompu");
        }
        segments.push(Segment { marker, start: i, end, data_start: i + 4 });
        i = end;
    }
    let scan_start = i;
    Ok(Parsed { segments, scan_start, eoi_end: find_eoi(b, scan_start) })
}

/// Cherche la fin d'image (FFD9) en tenant compte des segments intermédiaires.
fn find_eoi(b: &[u8], mut j: usize) -> Option<usize> {
    while j + 1 < b.len() {
        if b[j] != 0xFF {
            j += 1;
            continue;
        }
        let n = b[j + 1];
        match n {
            0x00 | 0xFF | 0xD0..=0xD7 | 0x01 => j += 1,
            0xD9 => return Some(j + 2),
            _ => {
                if j + 4 > b.len() {
                    return None;
                }
                let len = u16::from_be_bytes([b[j + 2], b[j + 3]]) as usize;
                j += 2 + len.max(2);
            }
        }
    }
    None
}

fn classify(seg: &Segment, b: &[u8]) -> Kind {
    let data = &b[seg.data_start..seg.end];
    match seg.marker {
        0xE1 if data.starts_with(EXIF_SIG) => Kind::Exif,
        0xE1 if data.starts_with(XMP_SIG) => Kind::Xmp,
        0xE1 if data.starts_with(XMP_EXT_SIG) => Kind::XmpExt,
        0xED if data.starts_with(PS_SIG) => Kind::Iptc,
        0xE2 if data.starts_with(ICC_SIG) => Kind::Icc,
        0xFE => Kind::Comment,
        0xE0 if data.starts_with(b"JFIF\0") => Kind::Keep,
        0xEE => Kind::Keep, // Adobe : nécessaire au décodage des couleurs
        0xE0..=0xEF => Kind::OtherApp,
        _ => Kind::Keep,
    }
}

fn iptc_entries(data: &[u8]) -> Vec<Entry> {
    let mut out = Vec::new();
    let mut i = PS_SIG.len();
    let mut other_resources = 0;
    while i + 12 <= data.len() && &data[i..i + 4] == b"8BIM" {
        let id = u16::from_be_bytes([data[i + 4], data[i + 5]]);
        let name_len = data[i + 6] as usize;
        let mut p = i + 7 + name_len;
        if (name_len + 1) % 2 == 1 {
            p += 1;
        }
        if p + 4 > data.len() {
            break;
        }
        let size = u32::from_be_bytes([data[p], data[p + 1], data[p + 2], data[p + 3]]) as usize;
        let start = p + 4;
        let end = (start + size).min(data.len());
        if id == 0x0404 {
            out.extend(iim_entries(&data[start..end]));
        } else {
            other_resources += 1;
        }
        i = end + (size % 2);
    }
    if other_resources > 0 {
        out.push(Entry::new("Ressources Photoshop", format!("{} bloc(s) supplémentaire(s)", other_resources)));
    }
    out
}

fn iim_entries(d: &[u8]) -> Vec<Entry> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 5 <= d.len() && d[i] == 0x1C {
        let rec = d[i + 1];
        let ds = d[i + 2];
        let len = u16::from_be_bytes([d[i + 3], d[i + 4]]) as usize;
        let s = i + 5;
        let e = (s + len).min(d.len());
        if rec == 2 && ds != 0 {
            let name = match ds {
                5 => "Titre",
                25 => "Mot-clé",
                40 => "Instructions",
                55 => "Date de création",
                60 => "Heure de création",
                65 => "Programme d'origine",
                80 => "Auteur",
                85 => "Fonction de l'auteur",
                90 => "Ville",
                92 => "Lieu",
                95 => "Région",
                101 => "Pays",
                105 => "Titre de presse",
                110 => "Crédit",
                115 => "Source",
                116 => "Droits d'auteur",
                118 => "Contact",
                120 => "Légende",
                122 => "Rédacteur",
                _ => "",
            };
            let key = if name.is_empty() { format!("IPTC 2:{}", ds) } else { name.to_string() };
            out.push(Entry::new(key, String::from_utf8_lossy(&d[s..e]).into_owned()));
        }
        i = e;
    }
    out
}

pub fn analyze(b: &[u8]) -> Result<(Vec<Group>, Vec<String>)> {
    let p = parse(b)?;
    let mut groups: Vec<Group> = Vec::new();
    let mut xmp_entries = Vec::new();
    let mut iptc = Vec::new();
    let mut comments = Vec::new();
    let mut icc = 0usize;
    let mut other = Vec::new();

    for seg in &p.segments {
        let data = &b[seg.data_start..seg.end];
        match classify(seg, b) {
            Kind::Exif => groups.extend(exifmeta::analyze(&data[EXIF_SIG.len()..])),
            Kind::Xmp => xmp_entries.extend(xmp::entries(&data[XMP_SIG.len()..])),
            Kind::XmpExt => xmp_entries.push(Entry::new("Extension XMP", format!("{} octets", data.len()))),
            Kind::Iptc => iptc.extend(iptc_entries(data)),
            Kind::Icc => icc += data.len(),
            Kind::Comment => comments.push(Entry::new("Commentaire", String::from_utf8_lossy(data).into_owned())),
            Kind::OtherApp => {
                let sig: String = data
                    .iter()
                    .take_while(|&&c| c != 0 && c.is_ascii_graphic())
                    .take(24)
                    .map(|&c| c as char)
                    .collect();
                let name = if sig.is_empty() { "sans signature".to_string() } else { sig };
                other.push(Entry::new(format!("APP{} ({})", seg.marker - 0xE0, name), format!("{} octets", data.len())));
            }
            Kind::Keep => {}
        }
    }

    if !xmp_entries.is_empty() {
        let risk = xmp::risk_of(&xmp_entries);
        groups.push(
            Group::new(
                "xmp",
                "Métadonnées XMP",
                "Bloc Adobe ajouté par les logiciels de retouche : auteur, logiciels, historique des modifications…",
                risk,
            )
            .entries(xmp_entries),
        );
    }
    if !iptc.is_empty() {
        groups.push(
            Group::new(
                "iptc",
                "IPTC / Photoshop",
                "Informations de presse : auteur, légende, lieu, droits, mots-clés.",
                Risk::High,
            )
            .entries(iptc),
        );
    }
    if !comments.is_empty() {
        groups.push(
            Group::new("comment", "Commentaires JPEG", "Texte libre inséré dans le fichier.", Risk::Medium)
                .entries(comments),
        );
    }
    if !other.is_empty() {
        groups.push(
            Group::new(
                "app.other",
                "Autres blocs de données",
                "Segments ajoutés par des appareils ou logiciels (aperçus, cartes de profondeur, données propriétaires).",
                Risk::Medium,
            )
            .entries(other),
        );
    }
    if let Some(eoi) = p.eoi_end {
        if b.len() > eoi + 16 {
            groups.push(
                Group::new(
                    "trailer",
                    "Données cachées après l'image",
                    "Des octets sont ajoutés après la fin de l'image (vidéo « Live », seconde image, données du téléphone…).",
                    Risk::High,
                )
                .entries(vec![Entry::new("Taille", format!("{} octets", b.len() - eoi))]),
            );
        }
    }
    if icc > 0 {
        groups.push(
            Group::new(
                "icc",
                "Profil de couleur ICC",
                "Décrit comment afficher les couleurs. Rarement sensible.",
                Risk::Low,
            )
            .not_recommended()
            .note("Le supprimer peut légèrement modifier le rendu des couleurs.")
            .entries(vec![Entry::new("Taille", format!("{} octets", icc))]),
        );
    }
    Ok((groups, Vec::new()))
}

pub fn clean(b: &[u8], sel: &HashSet<String>) -> Result<(Vec<u8>, Vec<String>)> {
    let p = parse(b)?;
    let mut out = Vec::with_capacity(b.len());
    let mut notes = Vec::new();
    out.extend_from_slice(&b[0..2]);
    let any_exif = sel.iter().any(|s| s.starts_with("exif."));

    for seg in &p.segments {
        let raw = &b[seg.start..seg.end];
        let data = &b[seg.data_start..seg.end];
        let keep = match classify(seg, b) {
            Kind::Exif => {
                if any_exif {
                    match exifmeta::rebuild(&data[EXIF_SIG.len()..], &|c| sel.contains(c)) {
                        Ok(None) => {}
                        Ok(Some(tiff)) => {
                            let len = tiff.len() + EXIF_SIG.len() + 2;
                            if len <= 0xFFFF {
                                out.extend_from_slice(&[0xFF, 0xE1]);
                                out.extend_from_slice(&(len as u16).to_be_bytes());
                                out.extend_from_slice(EXIF_SIG);
                                out.extend_from_slice(&tiff);
                            } else {
                                notes.push("Bloc EXIF trop grand pour être reconstruit : supprimé en entier.".into());
                            }
                        }
                        Err(e) => notes.push(format!(
                            "Le bloc EXIF n'a pas pu être reconstruit partiellement ({}), il a été supprimé en entier.",
                            e
                        )),
                    }
                    false
                } else {
                    true
                }
            }
            Kind::Xmp | Kind::XmpExt => !sel.contains("xmp"),
            Kind::Iptc => !sel.contains("iptc"),
            Kind::Icc => !sel.contains("icc"),
            Kind::Comment => !sel.contains("comment"),
            Kind::OtherApp => !sel.contains("app.other"),
            Kind::Keep => true,
        };
        if keep {
            out.extend_from_slice(raw);
        }
    }
    let end = match (p.eoi_end, sel.contains("trailer")) {
        (Some(e), true) => e,
        _ => b.len(),
    };
    out.extend_from_slice(&b[p.scan_start..end]);
    Ok((out, notes))
}
