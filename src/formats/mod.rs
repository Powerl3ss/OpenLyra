//! Détection du format et aiguillage vers le bon module.

pub mod jpeg;
pub mod odf;
pub mod ooxml;
pub mod pdf;
pub mod png;
pub mod zipdoc;

use crate::model::{FileKind, Group, Report};
use anyhow::{bail, Context, Result};
use std::collections::HashSet;
use std::path::Path;

pub const SUPPORTED: &[&str] = &[
    "jpg", "jpeg", "png", "pdf", "docx", "docm", "dotx", "xlsx", "xlsm", "pptx", "pptm", "odt", "ods", "odp", "odg",
    "ott",
];

pub fn detect(b: &[u8]) -> Result<FileKind> {
    if b.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Ok(FileKind::Jpeg);
    }
    if b.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Ok(FileKind::Png);
    }
    if b.len() > 5 && b[..b.len().min(1024)].windows(5).any(|w| w == b"%PDF-") {
        return Ok(FileKind::Pdf);
    }
    if b.starts_with(b"PK\x03\x04") {
        let entries = zipdoc::read(b).context("Archive illisible")?;
        if zipdoc::find(&entries, "[Content_Types].xml").is_some() {
            if entries.iter().any(|e| e.name.starts_with("word/")) {
                return Ok(FileKind::Word);
            }
            if entries.iter().any(|e| e.name.starts_with("xl/")) {
                return Ok(FileKind::Excel);
            }
            if entries.iter().any(|e| e.name.starts_with("ppt/")) {
                return Ok(FileKind::PowerPoint);
            }
        }
        if zipdoc::find(&entries, "mimetype").is_some() {
            return Ok(odf::kind_of(b));
        }
        bail!("Archive zip non reconnue comme document Office ou LibreOffice");
    }
    bail!("Format non pris en charge. Formats acceptés : JPEG, PNG, PDF, Word, Excel, PowerPoint, LibreOffice.")
}

pub fn analyze_bytes(b: &[u8]) -> Result<(FileKind, Vec<Group>, Vec<String>)> {
    let kind = detect(b)?;
    let (groups, warnings) = match kind {
        FileKind::Jpeg => jpeg::analyze(b)?,
        FileKind::Png => png::analyze(b)?,
        FileKind::Pdf => pdf::analyze(b)?,
        FileKind::Word | FileKind::Excel | FileKind::PowerPoint => ooxml::analyze(b, kind)?,
        _ => odf::analyze(b)?,
    };
    Ok((kind, groups, warnings))
}

pub fn analyze_file(path: &Path) -> Result<Report> {
    let bytes = std::fs::read(path).with_context(|| format!("Impossible de lire {}", path.display()))?;
    let (kind, groups, warnings) = analyze_bytes(&bytes)?;
    Ok(Report { path: path.to_path_buf(), kind, size: bytes.len() as u64, groups, warnings })
}

pub fn clean_bytes(b: &[u8], kind: FileKind, sel: &HashSet<String>) -> Result<(Vec<u8>, Vec<String>)> {
    match kind {
        FileKind::Jpeg => jpeg::clean(b, sel),
        FileKind::Png => png::clean(b, sel),
        FileKind::Pdf => pdf::clean(b, sel),
        FileKind::Word | FileKind::Excel | FileKind::PowerPoint => ooxml::clean(b, kind, sel),
        _ => odf::clean(b, sel),
    }
}

/// Analyse d'une image intégrée dans un document.
pub fn analyze_image(b: &[u8]) -> Option<Vec<Group>> {
    match detect(b).ok()? {
        FileKind::Jpeg => jpeg::analyze(b).ok().map(|r| r.0),
        FileKind::Png => png::analyze(b).ok().map(|r| r.0),
        _ => None,
    }
}

/// Nettoie une image intégrée avec la sélection recommandée.
pub fn clean_image_recommended(b: &[u8]) -> Option<Vec<u8>> {
    let kind = detect(b).ok()?;
    let groups = analyze_image(b)?;
    let sel: HashSet<String> = groups.iter().filter(|g| g.recommended && g.removable).map(|g| g.id.clone()).collect();
    if sel.is_empty() {
        return None;
    }
    clean_bytes(b, kind, &sel).ok().map(|r| r.0)
}

/// Chemin de sortie par défaut : « nom_nettoye.ext » à côté de l'original.
pub fn default_output(path: &Path) -> std::path::PathBuf {
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "document".into());
    let ext = path.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    let dir = path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
    let mut candidate = dir.join(format!("{}_nettoye{}", stem, ext));
    let mut n = 2;
    while candidate.exists() {
        candidate = dir.join(format!("{}_nettoye_{}{}", stem, n, ext));
        n += 1;
    }
    candidate
}
