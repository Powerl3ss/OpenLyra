//! PDF : dictionnaire Info, XMP, identifiant, annotations, révisions.

use crate::model::{Entry, Group, Risk};
use crate::xmp;
use anyhow::{anyhow, Result};
use lopdf::{Dictionary, Document, Object, ObjectId};
use std::collections::HashSet;

const AUTHOR: &[&str] = &["Author"];
const DESC: &[&str] = &["Title", "Subject", "Keywords"];
const SOFTWARE: &[&str] = &["Creator", "Producer"];
const DATES: &[&str] = &["CreationDate", "ModDate"];
const STANDARD: &[&str] = &["Author", "Title", "Subject", "Keywords", "Creator", "Producer", "CreationDate", "ModDate", "Trapped"];

fn bucket(key: &str) -> &'static str {
    if AUTHOR.contains(&key) {
        "pdf.author"
    } else if DESC.contains(&key) {
        "pdf.desc"
    } else if SOFTWARE.contains(&key) {
        "pdf.software"
    } else if DATES.contains(&key) {
        "pdf.dates"
    } else if key == "Trapped" {
        "pdf.software"
    } else {
        "pdf.custom"
    }
}

fn key_label(k: &str) -> String {
    match k {
        "Author" => "Auteur".into(),
        "Title" => "Titre".into(),
        "Subject" => "Sujet".into(),
        "Keywords" => "Mots-clés".into(),
        "Creator" => "Application d'origine".into(),
        "Producer" => "Producteur PDF".into(),
        "CreationDate" => "Date de création".into(),
        "ModDate" => "Date de modification".into(),
        "Trapped" => "Recouvrement (Trapped)".into(),
        _ => k.into(),
    }
}

pub fn decode_text(b: &[u8]) -> String {
    if b.len() >= 2 && b[0] == 0xFE && b[1] == 0xFF {
        let u: Vec<u16> = b[2..].chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
        String::from_utf16_lossy(&u)
    } else if b.starts_with(&[0xEF, 0xBB, 0xBF]) {
        String::from_utf8_lossy(&b[3..]).into_owned()
    } else {
        b.iter().map(|&c| c as char).collect()
    }
}

/// D:20240305142200+01'00' -> 05/03/2024 à 14:22:00 (+01:00)
fn pretty_date(s: &str) -> String {
    let t = s.trim().trim_start_matches("D:");
    let d: String = t.chars().take_while(|c| c.is_ascii_digit()).collect();
    if d.len() < 8 {
        return s.to_string();
    }
    let mut out = format!("{}/{}/{}", &d[6..8], &d[4..6], &d[0..4]);
    if d.len() >= 12 {
        out.push_str(&format!(" à {}:{}", &d[8..10], &d[10..12]));
        if d.len() >= 14 {
            out.push_str(&format!(":{}", &d[12..14]));
        }
    }
    let rest = &t[d.len()..];
    if rest.starts_with('Z') {
        out.push_str(" (UTC)");
    } else if rest.len() >= 3 && (rest.starts_with('+') || rest.starts_with('-')) {
        let tz: String = rest.chars().filter(|c| c.is_ascii_digit() || *c == '+' || *c == '-').collect();
        if tz.len() >= 5 {
            out.push_str(&format!(" ({}:{})", &tz[0..3], &tz[3..5]));
        }
    }
    out
}

fn resolve<'a>(doc: &'a Document, o: &'a Object) -> &'a Object {
    match o {
        Object::Reference(id) => doc.get_object(*id).unwrap_or(o),
        _ => o,
    }
}

fn obj_text(doc: &Document, o: &Object) -> String {
    match resolve(doc, o) {
        Object::String(b, _) => decode_text(b),
        Object::Name(n) => String::from_utf8_lossy(n).into_owned(),
        Object::Integer(i) => i.to_string(),
        Object::Real(r) => r.to_string(),
        Object::Boolean(b) => b.to_string(),
        Object::Array(a) => a.iter().map(|x| obj_text(doc, x)).collect::<Vec<_>>().join(", "),
        _ => "(valeur complexe)".into(),
    }
}

fn info_id(doc: &Document) -> Option<ObjectId> {
    doc.trailer.get(b"Info").ok()?.as_reference().ok()
}

fn info_dict(doc: &Document) -> Option<&Dictionary> {
    match doc.trailer.get(b"Info").ok()? {
        Object::Reference(id) => doc.get_object(*id).ok()?.as_dict().ok(),
        Object::Dictionary(d) => Some(d),
        _ => None,
    }
}

fn root_id(doc: &Document) -> Option<ObjectId> {
    doc.trailer.get(b"Root").ok()?.as_reference().ok()
}

fn stream_bytes(doc: &Document, o: &Object) -> Option<Vec<u8>> {
    let s = resolve(doc, o).as_stream().ok()?;
    Some(s.decompressed_content().unwrap_or_else(|_| s.content.clone()))
}

fn object_dict(o: &Object) -> Option<&Dictionary> {
    match o {
        Object::Dictionary(d) => Some(d),
        Object::Stream(s) => Some(&s.dict),
        _ => None,
    }
}

fn object_dict_mut(o: &mut Object) -> Option<&mut Dictionary> {
    match o {
        Object::Dictionary(d) => Some(d),
        Object::Stream(s) => Some(&mut s.dict),
        _ => None,
    }
}

const KEEP_ANNOTS: &[&str] = &["Link", "Widget"];

fn annot_label(sub: &str) -> &str {
    match sub {
        "Text" => "Note",
        "FreeText" => "Zone de texte",
        "Highlight" => "Surlignage",
        "Underline" => "Soulignement",
        "StrikeOut" => "Texte barré",
        "Squiggly" => "Soulignement ondulé",
        "Ink" => "Dessin à main levée",
        "Stamp" => "Tampon",
        "Caret" => "Insertion",
        "FileAttachment" => "Pièce jointe",
        "Square" | "Circle" | "Line" | "Polygon" | "PolyLine" => "Forme",
        "Popup" => "Fenêtre de note",
        _ => sub,
    }
}

fn load(bytes: &[u8]) -> Result<Document> {
    let doc = Document::load_mem(bytes).map_err(|e| anyhow!("PDF illisible : {}", e))?;
    if doc.is_encrypted() {
        return Err(anyhow!(
            "Ce PDF est chiffré ou protégé par mot de passe. Retire la protection avant de le nettoyer."
        ));
    }
    Ok(doc)
}

pub fn analyze(bytes: &[u8]) -> Result<(Vec<Group>, Vec<String>)> {
    let doc = load(bytes)?;
    let mut groups = Vec::new();
    let mut warnings = Vec::new();

    // Dictionnaire Info
    let mut buckets: Vec<(&str, Vec<Entry>)> = vec![
        ("pdf.author", vec![]),
        ("pdf.desc", vec![]),
        ("pdf.software", vec![]),
        ("pdf.dates", vec![]),
        ("pdf.custom", vec![]),
    ];
    if let Some(info) = info_dict(&doc) {
        for (k, v) in info.iter() {
            let key = String::from_utf8_lossy(k).into_owned();
            let mut val = obj_text(&doc, v);
            if val.trim().is_empty() {
                continue;
            }
            let b = bucket(&key);
            if b == "pdf.dates" {
                val = pretty_date(&val);
            }
            if let Some((_, list)) = buckets.iter_mut().find(|(id, _)| *id == b) {
                list.push(Entry::new(key_label(&key), val));
            }
        }
    }
    for (id, entries) in buckets {
        if entries.is_empty() {
            continue;
        }
        let g = match id {
            "pdf.author" => Group::new(id, "Auteur", "Nom de la personne qui a créé le document.", Risk::High),
            "pdf.desc" => Group::new(
                id,
                "Titre, sujet et mots-clés",
                "Champs descriptifs des propriétés du PDF. Le titre peut différer du nom du fichier.",
                Risk::Medium,
            ),
            "pdf.software" => Group::new(
                id,
                "Logiciels utilisés",
                "Application d'origine et outil de conversion en PDF, avec leurs versions.",
                Risk::Medium,
            ),
            "pdf.dates" => Group::new(id, "Dates", "Dates de création et de dernière modification.", Risk::Medium),
            _ => Group::new(
                id,
                "Propriétés personnalisées",
                "Champs ajoutés par un logiciel ou une organisation (chemins, identifiants, noms…).",
                Risk::High,
            ),
        };
        groups.push(g.entries(entries));
    }

    // XMP du document
    if let Some(rid) = root_id(&doc) {
        if let Ok(cat) = doc.get_dictionary(rid) {
            if let Ok(m) = cat.get(b"Metadata") {
                if let Some(bytes) = stream_bytes(&doc, m) {
                    let e = xmp::entries(&bytes);
                    let r = xmp::risk_of(&e);
                    groups.push(
                        Group::new(
                            "pdf.xmp",
                            "Métadonnées XMP du document",
                            "Copie détaillée des propriétés (auteur, logiciels, identifiants, historique).",
                            r,
                        )
                        .entries(e),
                    );
                }
            }
        }
    }

    // XMP des objets internes (images, pages…)
    let root = root_id(&doc);
    let mut inner = Vec::new();
    for (id, obj) in &doc.objects {
        if Some(*id) == root {
            continue;
        }
        if let Some(d) = object_dict(obj) {
            if let Ok(m) = d.get(b"Metadata") {
                let kind = d
                    .get(b"Subtype")
                    .or_else(|_| d.get(b"Type"))
                    .ok()
                    .and_then(|n| n.as_name().ok())
                    .map(|n| String::from_utf8_lossy(n).into_owned())
                    .unwrap_or_else(|| "objet".into());
                let mut summary = String::from("bloc XMP");
                if inner.len() < 20 {
                    if let Some(b) = stream_bytes(&doc, m) {
                        let es = xmp::entries(&b);
                        summary = es
                            .iter()
                            .take(4)
                            .map(|e| format!("{} : {}", e.key, e.value))
                            .collect::<Vec<_>>()
                            .join(" | ");
                    }
                }
                inner.push(Entry::new(format!("Objet {} ({})", id.0, kind), summary));
            }
        }
    }
    if !inner.is_empty() {
        groups.push(
            Group::new(
                "pdf.xmp_objects",
                "Métadonnées des images et objets intégrés",
                "Les images ou pages importées gardent parfois leurs propres métadonnées XMP.",
                Risk::Medium,
            )
            .entries(inner),
        );
    }

    // Annotations
    let mut annots = Vec::new();
    for (num, pid) in doc.get_pages() {
        for (_, d) in page_annots(&doc, pid) {
            let sub = d
                .get(b"Subtype")
                .ok()
                .and_then(|n| n.as_name().ok())
                .map(|n| String::from_utf8_lossy(n).into_owned())
                .unwrap_or_default();
            if KEEP_ANNOTS.contains(&sub.as_str()) || sub == "Popup" {
                continue;
            }
            let author = d.get(b"T").map(|o| obj_text(&doc, o)).unwrap_or_default();
            let text = d.get(b"Contents").map(|o| obj_text(&doc, o)).unwrap_or_default();
            let mut v = String::new();
            if !author.is_empty() {
                v.push_str(&format!("Auteur : {}", author));
            }
            if !text.is_empty() {
                if !v.is_empty() {
                    v.push_str(" | ");
                }
                v.push_str(&text);
            }
            if v.is_empty() {
                v = "sans texte".into();
            }
            annots.push(Entry::new(format!("Page {} · {}", num, annot_label(&sub)), v));
        }
    }
    if !annots.is_empty() {
        groups.push(
            Group::new(
                "pdf.annots",
                "Annotations et commentaires",
                "Notes, surlignages et dessins ajoutés au document, souvent avec le nom de leur auteur.",
                Risk::High,
            )
            .not_recommended()
            .note("Les liens et les champs de formulaire sont conservés.")
            .entries(annots),
        );
    }

    // Identifiant du document
    if let Ok(Object::Array(a)) = doc.trailer.get(b"ID") {
        let ids: Vec<String> = a
            .iter()
            .filter_map(|o| o.as_str().ok())
            .map(|b| b.iter().map(|x| format!("{:02X}", x)).collect::<String>())
            .collect();
        if !ids.is_empty() {
            groups.push(
                Group::new(
                    "pdf.id",
                    "Identifiant unique du fichier",
                    "Empreinte qui permet de relier ce fichier à ses autres versions.",
                    Risk::Low,
                )
                .entries(ids.into_iter().enumerate().map(|(i, s)| Entry::new(format!("ID {}", i + 1), s)).collect()),
            );
        }
    }

    // Révisions incrémentales
    let eofs = count(bytes, b"%%EOF");
    if eofs > 1 {
        groups.push(
            Group::new(
                "pdf.revisions",
                "Versions antérieures cachées",
                "Le fichier contient plusieurs enregistrements successifs : d'anciennes versions du contenu et des propriétés peuvent encore être lues.",
                Risk::High,
            )
            .info_only()
            .note("Elles sont toujours supprimées : le nettoyage réécrit entièrement le PDF.")
            .entries(vec![Entry::new("Enregistrements détectés", eofs.to_string())]),
        );
    }

    if doc.catalog().map(|c| c.has(b"Names")).unwrap_or(false) {
        let has_files = doc
            .catalog()
            .ok()
            .and_then(|c| c.get(b"Names").ok())
            .map(|n| resolve(&doc, n))
            .and_then(|n| n.as_dict().ok())
            .map(|d| d.has(b"EmbeddedFiles"))
            .unwrap_or(false);
        if has_files {
            warnings.push("Ce PDF contient des fichiers joints. Leurs propres métadonnées ne sont pas nettoyées.".into());
        }
    }
    Ok((groups, warnings))
}

fn count(h: &[u8], n: &[u8]) -> usize {
    h.windows(n.len()).filter(|w| *w == n).count()
}

fn page_annots(doc: &Document, pid: ObjectId) -> Vec<(ObjectId, &Dictionary)> {
    let mut v = Vec::new();
    let Ok(page) = doc.get_dictionary(pid) else { return v };
    let Ok(a) = page.get(b"Annots") else { return v };
    if let Object::Array(arr) = resolve(doc, a) {
        for o in arr {
            if let Object::Reference(id) = o {
                if let Ok(d) = doc.get_dictionary(*id) {
                    v.push((*id, d));
                }
            }
        }
    }
    v
}

pub fn clean(bytes: &[u8], sel: &HashSet<String>) -> Result<(Vec<u8>, Vec<String>)> {
    let mut doc = load(bytes)?;
    let mut notes = vec!["Le PDF a été entièrement réécrit : les versions antérieures qu'il contenait ont disparu.".to_string()];

    // Info
    let remove_key = |k: &str| sel.contains(bucket(k));
    let mut info_empty = false;
    if let Some(id) = info_id(&doc) {
        if let Ok(d) = doc.get_object_mut(id).and_then(|o| o.as_dict_mut()) {
            let keys: Vec<Vec<u8>> = d.iter().map(|(k, _)| k.clone()).collect();
            for k in keys {
                if remove_key(&String::from_utf8_lossy(&k)) {
                    d.remove(&k);
                }
            }
            info_empty = d.is_empty();
        }
    } else if let Ok(Object::Dictionary(d)) = doc.trailer.get_mut(b"Info") {
        let keys: Vec<Vec<u8>> = d.iter().map(|(k, _)| k.clone()).collect();
        for k in keys {
            if remove_key(&String::from_utf8_lossy(&k)) {
                d.remove(&k);
            }
        }
        info_empty = d.is_empty();
    }
    if info_empty {
        doc.trailer.remove(b"Info");
    }
    let _ = STANDARD;

    let root = root_id(&doc);
    if sel.contains("pdf.xmp") {
        if let Some(rid) = root {
            if let Ok(d) = doc.get_object_mut(rid).and_then(|o| o.as_dict_mut()) {
                d.remove(b"Metadata");
            }
        }
    }
    if sel.contains("pdf.xmp_objects") {
        let ids: Vec<ObjectId> = doc.objects.keys().copied().filter(|id| Some(*id) != root).collect();
        for id in ids {
            if let Some(o) = doc.objects.get_mut(&id) {
                if let Some(d) = object_dict_mut(o) {
                    d.remove(b"Metadata");
                }
            }
        }
    }

    if sel.contains("pdf.annots") {
        let pages: Vec<ObjectId> = doc.get_pages().values().copied().collect();
        let mut removed = 0;
        for pid in pages {
            let keep: Option<Vec<Object>> = {
                let list = page_annots(&doc, pid);
                if list.is_empty() {
                    None
                } else {
                    let keep_ids: Vec<Object> = list
                        .iter()
                        .filter(|(_, d)| {
                            d.get(b"Subtype")
                                .ok()
                                .and_then(|n| n.as_name().ok())
                                .map(|n| KEEP_ANNOTS.iter().any(|k| k.as_bytes() == n))
                                .unwrap_or(false)
                        })
                        .map(|(id, _)| Object::Reference(*id))
                        .collect();
                    removed += list.len() - keep_ids.len();
                    Some(keep_ids)
                }
            };
            if let Some(k) = keep {
                // Annots peut être un tableau direct ou une référence
                let target = doc
                    .get_dictionary(pid)
                    .ok()
                    .and_then(|p| p.get(b"Annots").ok())
                    .and_then(|a| a.as_reference().ok());
                match target {
                    Some(aid) => {
                        if let Some(o) = doc.objects.get_mut(&aid) {
                            *o = Object::Array(k);
                        }
                    }
                    None => {
                        if let Ok(p) = doc.get_object_mut(pid).and_then(|o| o.as_dict_mut()) {
                            if k.is_empty() {
                                p.remove(b"Annots");
                            } else {
                                p.set("Annots", Object::Array(k));
                            }
                        }
                    }
                }
            }
        }
        if removed > 0 {
            notes.push(format!("{} annotation(s) supprimée(s).", removed));
        }
    }

    if sel.contains("pdf.id") {
        doc.trailer.remove(b"ID");
    }

    doc.prune_objects();
    let mut out = Vec::new();
    doc.save_to(&mut out).map_err(|e| anyhow!("Échec de l'écriture du PDF : {}", e))?;
    Ok((out, notes))
}
