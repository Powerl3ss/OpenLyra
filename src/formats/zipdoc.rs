//! Conteneur zip commun aux formats Office et OpenDocument, et définitions
//! des groupes partagés.

use crate::model::{Entry, Group, Risk};
use anyhow::Result;
use std::collections::HashSet;
use std::io::{Cursor, Read, Write};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipArchive, ZipWriter};

pub struct ZipEntry {
    pub name: String,
    pub data: Vec<u8>,
    pub stored: bool,
    pub is_dir: bool,
}

pub fn read(bytes: &[u8]) -> Result<Vec<ZipEntry>> {
    let mut ar = ZipArchive::new(Cursor::new(bytes))?;
    let mut out = Vec::with_capacity(ar.len());
    for i in 0..ar.len() {
        let mut f = ar.by_index(i)?;
        let mut data = Vec::new();
        let is_dir = f.is_dir();
        if !is_dir {
            f.read_to_end(&mut data)?;
        }
        out.push(ZipEntry {
            name: f.name().to_string(),
            stored: f.compression() == CompressionMethod::Stored,
            data,
            is_dir,
        });
    }
    Ok(out)
}

/// Réécrit l'archive. Les dates des fichiers internes sont remises au
/// 01/01/1980 et le commentaire d'archive est supprimé.
pub fn write(entries: &[ZipEntry]) -> Result<Vec<u8>> {
    let mut w = ZipWriter::new(Cursor::new(Vec::new()));
    for e in entries {
        let opts = SimpleFileOptions::default()
            .compression_method(if e.stored { CompressionMethod::Stored } else { CompressionMethod::Deflated })
            .last_modified_time(DateTime::default());
        if e.is_dir {
            w.add_directory(e.name.clone(), opts)?;
        } else {
            w.start_file(e.name.clone(), opts)?;
            w.write_all(&e.data)?;
        }
    }
    Ok(w.finish()?.into_inner())
}

pub fn find<'a>(entries: &'a [ZipEntry], name: &str) -> Option<&'a ZipEntry> {
    entries.iter().find(|e| e.name == name)
}

pub fn office_group(id: &str) -> Group {
    match id {
        "people" => Group::new(
            id,
            "Auteurs et personnes",
            "Noms des personnes qui ont créé, modifié ou imprimé le document.",
            Risk::High,
        ),
        "org" => Group::new(
            id,
            "Organisation",
            "Société, responsable et adresse de base des liens.",
            Risk::High,
        ),
        "dates" => Group::new(id, "Dates", "Dates de création, de dernière modification et d'impression.", Risk::Medium),
        "desc" => Group::new(
            id,
            "Titre, sujet et mots-clés",
            "Champs descriptifs des propriétés du fichier (invisibles dans le contenu).",
            Risk::Medium,
        ),
        "software" => Group::new(
            id,
            "Logiciel et modèle",
            "Application utilisée, sa version et le modèle d'origine (son chemin peut contenir ton nom d'utilisateur).",
            Risk::Medium,
        ),
        "history" => Group::new(
            id,
            "Historique d'édition",
            "Nombre d'enregistrements et temps total passé à modifier le document.",
            Risk::Medium,
        ),
        "stats" => Group::new(id, "Statistiques", "Nombre de pages, de mots, de caractères…", Risk::Low)
            .not_recommended(),
        "custom" => Group::new(
            id,
            "Propriétés personnalisées",
            "Champs ajoutés par l'auteur, une entreprise ou un logiciel de gestion documentaire.",
            Risk::High,
        ),
        "thumb" => Group::new(
            id,
            "Miniature d'aperçu",
            "Image de la première page enregistrée dans le fichier, qui peut montrer une version antérieure.",
            Risk::Medium,
        ),
        "printer" => Group::new(
            id,
            "Imprimante",
            "Nom et réglages de l'imprimante utilisée (souvent le nom du poste ou du réseau).",
            Risk::Medium,
        ),
        "media" => Group::new(
            id,
            "Métadonnées des images intégrées",
            "Les photos insérées gardent leurs propres métadonnées (GPS, appareil, auteur…). Le nettoyage applique la sélection recommandée à chaque image.",
            Risk::Medium,
        ),
        "comments" => Group::new(
            id,
            "Commentaires",
            "Commentaires laissés dans le document, avec le nom de leur auteur et la date.",
            Risk::High,
        )
        .not_recommended()
        .note("Supprime les commentaires eux-mêmes, pas seulement leurs auteurs."),
        "revisions" => Group::new(
            id,
            "Suivi des modifications",
            "Modifications non acceptées : le texte supprimé reste lisible, avec le nom de l'auteur et la date.",
            Risk::High,
        )
        .not_recommended()
        .note("Le nettoyage accepte toutes les modifications : les ajouts sont gardés et le texte supprimé disparaît définitivement."),
        _ => Group::new(id, id, "", Risk::Medium),
    }
}

pub const ORDER: &[&str] = &[
    "people", "org", "revisions", "comments", "custom", "media", "dates", "desc", "software", "history", "printer",
    "thumb", "stats",
];

/// Résumé lisible des métadonnées d'une image intégrée.
pub fn media_entry(name: &str, data: &[u8]) -> Option<(Entry, Risk)> {
    let groups = super::analyze_image(data)?;
    let relevant: Vec<_> = groups.iter().filter(|g| g.recommended && !g.entries.is_empty()).collect();
    if relevant.is_empty() {
        return None;
    }
    let risk = relevant.iter().map(|g| g.risk).max().unwrap_or(Risk::Low);
    let mut summary = relevant.iter().map(|g| g.title.clone()).collect::<Vec<_>>().join(", ");
    if let Some(gps) = groups.iter().find(|g| g.id == "exif.gps") {
        if let Some(pos) = gps.entries.first() {
            summary.push_str(&format!(" | Position {}", pos.value));
        }
    }
    let short = name.rsplit('/').next().unwrap_or(name);
    Some((Entry::new(short, summary), risk))
}

pub fn is_image(name: &str) -> bool {
    let l = name.to_ascii_lowercase();
    l.ends_with(".jpg") || l.ends_with(".jpeg") || l.ends_with(".png")
}

/// Assemble les groupes dans l'ordre d'affichage.
pub fn assemble(mut map: std::collections::BTreeMap<String, Vec<Entry>>, risks: &[(String, Risk)]) -> Vec<Group> {
    let mut out = Vec::new();
    for id in ORDER {
        if let Some(entries) = map.remove(*id) {
            if entries.is_empty() {
                continue;
            }
            let mut g = office_group(id).entries(entries);
            if let Some((_, r)) = risks.iter().find(|(i, _)| i == id) {
                g.risk = *r;
            }
            out.push(g);
        }
    }
    out
}

pub fn clean_media(entries: &mut [ZipEntry], sel: &HashSet<String>, notes: &mut Vec<String>, prefix: &[&str]) {
    if !sel.contains("media") {
        return;
    }
    let mut n = 0;
    for e in entries.iter_mut() {
        if prefix.iter().any(|p| e.name.starts_with(p)) && is_image(&e.name) {
            if let Some(clean) = super::clean_image_recommended(&e.data) {
                if clean.len() != e.data.len() || clean != e.data {
                    e.data = clean;
                    n += 1;
                }
            }
        }
    }
    if n > 0 {
        notes.push(format!("{} image(s) intégrée(s) nettoyée(s).", n));
    }
}

/// Décode le nom d'imprimante stocké dans une structure DEVMODE Windows.
pub fn devmode_name(b: &[u8]) -> Option<String> {
    if b.len() < 64 {
        return None;
    }
    let u: Vec<u16> = b[..64]
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .take_while(|&c| c != 0)
        .collect();
    let s = String::from_utf16_lossy(&u);
    if s.chars().all(|c| !c.is_control()) && !s.trim().is_empty() {
        Some(s)
    } else {
        None
    }
}
