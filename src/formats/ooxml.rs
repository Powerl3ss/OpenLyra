//! Microsoft Office (docx, xlsx, pptx et variantes).

use super::zipdoc;
use crate::model::{Entry, FileKind, Group, Risk};
use crate::xml::{self, Action};
use anyhow::Result;
use std::collections::{BTreeMap, HashSet};

const CORE: &[(&str, &str, &str)] = &[
    ("creator", "people", "Auteur"),
    ("lastModifiedBy", "people", "Modifié en dernier par"),
    ("created", "dates", "Date de création"),
    ("modified", "dates", "Date de dernière modification"),
    ("lastPrinted", "dates", "Dernière impression"),
    ("title", "desc", "Titre"),
    ("subject", "desc", "Sujet"),
    ("description", "desc", "Commentaires"),
    ("keywords", "desc", "Mots-clés"),
    ("category", "desc", "Catégorie"),
    ("contentStatus", "desc", "Statut"),
    ("identifier", "desc", "Identifiant"),
    ("language", "desc", "Langue"),
    ("version", "desc", "Version"),
    ("revision", "history", "Numéro de révision (nombre d'enregistrements)"),
];

const APP: &[(&str, &str, &str)] = &[
    ("Company", "org", "Société"),
    ("Manager", "org", "Responsable"),
    ("HyperlinkBase", "org", "Base des liens hypertexte"),
    ("Application", "software", "Application"),
    ("AppVersion", "software", "Version de l'application"),
    ("Template", "software", "Modèle"),
    ("TotalTime", "history", "Temps total d'édition (minutes)"),
    ("Pages", "stats", "Pages"),
    ("Words", "stats", "Mots"),
    ("Characters", "stats", "Caractères"),
    ("CharactersWithSpaces", "stats", "Caractères (espaces compris)"),
    ("Lines", "stats", "Lignes"),
    ("Paragraphs", "stats", "Paragraphes"),
    ("Slides", "stats", "Diapositives"),
    ("Notes", "stats", "Notes"),
    ("HiddenSlides", "stats", "Diapositives masquées"),
    ("MMClips", "stats", "Clips multimédias"),
    ("PresentationFormat", "stats", "Format de présentation"),
];

const REVISION_DROP: &[&str] = &[
    "del", "moveFrom", "rPrChange", "pPrChange", "sectPrChange", "tblPrChange", "trPrChange", "tcPrChange",
    "tblGridChange", "numberingChange", "moveFromRangeStart", "moveFromRangeEnd", "moveToRangeStart",
    "moveToRangeEnd", "cellDel", "cellIns", "cellMerge",
];
const COMMENT_MARKERS: &[&str] = &["commentRangeStart", "commentRangeEnd", "commentReference"];

fn is_word_body(name: &str) -> bool {
    name.starts_with("word/")
        && !name.contains("/_rels/")
        && name.ends_with(".xml")
        && (name == "word/document.xml"
            || name.starts_with("word/header")
            || name.starts_with("word/footer")
            || name == "word/footnotes.xml"
            || name == "word/endnotes.xml")
}

fn is_thumb(name: &str) -> bool {
    name.starts_with("docProps/thumbnail")
}

fn is_printer(name: &str) -> bool {
    name.contains("/printerSettings/")
}

pub fn analyze(bytes: &[u8], kind: FileKind) -> Result<(Vec<Group>, Vec<String>)> {
    let entries = zipdoc::read(bytes)?;
    let mut map: BTreeMap<String, Vec<Entry>> = BTreeMap::new();
    let mut risks: Vec<(String, Risk)> = Vec::new();
    let mut warnings = Vec::new();
    let mut extra_groups = Vec::new();
    let push = |map: &mut BTreeMap<String, Vec<Entry>>, g: &str, e: Entry| map.entry(g.to_string()).or_default().push(e);

    let simple = |map: &mut BTreeMap<String, Vec<Entry>>, part: &str, table: &[(&str, &str, &str)]| {
        if let Some(e) = zipdoc::find(&entries, part) {
            if let Ok(t) = xml::parse(&e.data) {
                if let Some(root) = t.root() {
                    for c in &root.children {
                        if let Some((_, g, label)) = table.iter().find(|(n, _, _)| *n == c.local()) {
                            let mut v = c.all_text();
                            if v.is_empty() {
                                continue;
                            }
                            if *g == "dates" {
                                v = xml::pretty_iso_date(&v);
                            }
                            map.entry(g.to_string()).or_default().push(Entry::new(*label, v));
                        }
                    }
                }
            }
        }
    };
    simple(&mut map, "docProps/core.xml", CORE);
    simple(&mut map, "docProps/app.xml", APP);

    // Propriétés personnalisées
    if let Some(e) = zipdoc::find(&entries, "docProps/custom.xml") {
        if let Ok(t) = xml::parse(&e.data) {
            for p in t.find_all("property") {
                let name = p.attr("name").unwrap_or("?").to_string();
                push(&mut map, "custom", Entry::new(name, p.all_text()));
            }
        }
    }

    // Modèle attaché (Word)
    if let Some(e) = zipdoc::find(&entries, "word/_rels/settings.xml.rels") {
        if let Ok(t) = xml::parse(&e.data) {
            for r in t.find_all("Relationship") {
                if r.attr("Type").map(|t| t.ends_with("/attachedTemplate")).unwrap_or(false) {
                    push(&mut map, "software", Entry::new("Chemin du modèle attaché", r.attr("Target").unwrap_or("")));
                }
            }
        }
    }

    for e in &entries {
        if is_thumb(&e.name) {
            push(&mut map, "thumb", Entry::new(e.name.clone(), crate::model::human_size(e.data.len() as u64)));
        }
        if is_printer(&e.name) {
            let v = zipdoc::devmode_name(&e.data).unwrap_or_else(|| "nom illisible".into());
            push(&mut map, "printer", Entry::new("Imprimante", v));
        }
        if (e.name.starts_with("word/media/") || e.name.starts_with("xl/media/") || e.name.starts_with("ppt/media/"))
            && zipdoc::is_image(&e.name)
        {
            if let Some((entry, r)) = zipdoc::media_entry(&e.name, &e.data) {
                push(&mut map, "media", entry);
                if r == Risk::High {
                    risks.retain(|(i, _)| i != "media");
                    risks.push(("media".into(), Risk::High));
                }
            }
        }
        if e.name.starts_with("customXml/item") && e.name.ends_with(".xml") && !e.name.contains("Props") {
            if let Ok(t) = xml::parse(&e.data) {
                if let Some(r) = t.root() {
                    if r.local() != "Sources" && !r.children.is_empty() {
                        warnings.push(format!(
                            "Données XML personnalisées présentes ({}), souvent ajoutées par SharePoint. Elles ne sont pas modifiées.",
                            e.name
                        ));
                    }
                }
            }
        }
    }

    match kind {
        FileKind::Word => {
            if let Some(e) = zipdoc::find(&entries, "word/comments.xml") {
                if let Ok(t) = xml::parse(&e.data) {
                    for c in t.find_all("comment") {
                        let author = c.attr("author").unwrap_or("Inconnu");
                        let date = c.attr("date").map(xml::pretty_iso_date).unwrap_or_default();
                        let key = if date.is_empty() { author.to_string() } else { format!("{} ({})", author, date) };
                        push(&mut map, "comments", Entry::new(key, c.all_text()));
                    }
                }
            }
            let mut authors: BTreeMap<String, (usize, usize, usize)> = BTreeMap::new();
            for e in entries.iter().filter(|e| is_word_body(&e.name)) {
                if let Ok(t) = xml::parse(&e.data) {
                    t.walk(&mut |n, _| {
                        if !n.name.starts_with("w:") {
                            return;
                        }
                        let slot = match n.local() {
                            "ins" | "moveTo" => 0,
                            "del" | "moveFrom" => 1,
                            "rPrChange" | "pPrChange" | "sectPrChange" | "tblPrChange" => 2,
                            _ => return,
                        };
                        let a = n.attr("author").unwrap_or("Inconnu").to_string();
                        let c = authors.entry(a).or_default();
                        match slot {
                            0 => c.0 += 1,
                            1 => c.1 += 1,
                            _ => c.2 += 1,
                        }
                    });
                }
            }
            for (a, (i, d, f)) in authors {
                push(
                    &mut map,
                    "revisions",
                    Entry::new(a, format!("{} ajout(s), {} suppression(s), {} changement(s) de mise en forme", i, d, f)),
                );
            }
        }
        FileKind::Excel | FileKind::PowerPoint => {
            let mut people: Vec<String> = Vec::new();
            let mut count = 0;
            for e in &entries {
                let n = &e.name;
                let is_comment = (n.starts_with("xl/comments") || n.starts_with("xl/threadedComments/") || n.starts_with("ppt/comments/"))
                    && n.ends_with(".xml");
                let is_people = n == "xl/persons/person.xml" || n == "ppt/commentAuthors.xml" || n == "ppt/authors.xml";
                if !is_comment && !is_people {
                    continue;
                }
                if let Ok(t) = xml::parse(&e.data) {
                    t.walk(&mut |node, _| match node.local() {
                        "author" if node.attr("name").is_none() => people.push(node.all_text()),
                        "author" | "cmAuthor" => people.push(node.attr("name").unwrap_or("").to_string()),
                        "person" => people.push(node.attr("displayName").unwrap_or("").to_string()),
                        "comment" | "cm" | "threadedComment" if is_comment => count += 1,
                        _ => {}
                    });
                }
            }
            people.retain(|p| !p.trim().is_empty());
            people.sort();
            people.dedup();
            if count > 0 || !people.is_empty() {
                let mut es = vec![Entry::new("Nombre de commentaires", count.to_string())];
                if !people.is_empty() {
                    es.push(Entry::new("Auteurs", people.join(", ")));
                }
                extra_groups.push(
                    zipdoc::office_group("comments")
                        .entries(es)
                        .info_only()
                        .note("Suppression automatique disponible pour Word et LibreOffice uniquement : supprime-les depuis l'application (Révision > Supprimer)."),
                );
            }
        }
        _ => {}
    }

    let mut groups = zipdoc::assemble(map, &risks);
    let pos = groups.iter().position(|g| g.risk != Risk::High).unwrap_or(groups.len());
    for g in extra_groups {
        groups.insert(pos, g);
    }
    Ok((groups, warnings))
}

pub fn clean(bytes: &[u8], kind: FileKind, sel: &HashSet<String>) -> Result<(Vec<u8>, Vec<String>)> {
    let mut entries = zipdoc::read(bytes)?;
    let mut notes = vec!["Dates internes de l'archive remises à zéro.".to_string()];

    let drop_core: HashSet<&str> = CORE.iter().filter(|(_, g, _)| sel.contains(*g)).map(|(n, _, _)| *n).collect();
    let drop_app: HashSet<&str> = APP.iter().filter(|(_, g, _)| sel.contains(*g)).map(|(n, _, _)| *n).collect();
    let rm_thumb = sel.contains("thumb");
    let rm_printer = sel.contains("printer");
    let rm_comments = sel.contains("comments") && kind == FileKind::Word;
    let rm_revisions = sel.contains("revisions") && kind == FileKind::Word;
    let rm_template = sel.contains("software");

    let removed: Vec<String> = entries
        .iter()
        .filter(|e| (rm_thumb && is_thumb(&e.name)) || (rm_printer && is_printer(&e.name)))
        .map(|e| e.name.clone())
        .collect();
    entries.retain(|e| !removed.contains(&e.name));
    let is_removed_target = |t: &str| {
        let t = t.trim_start_matches('/');
        removed.iter().any(|r| r == t || r.ends_with(t.trim_start_matches("../")))
    };

    for e in entries.iter_mut() {
        let name = e.name.clone();
        let new = if name == "docProps/core.xml" && !drop_core.is_empty() {
            Some(xml::filter(&e.data, |s| {
                if drop_core.contains(xml::local(s).as_str()) { Action::Drop } else { Action::Keep }
            })?)
        } else if name == "docProps/app.xml" && !drop_app.is_empty() {
            Some(xml::filter(&e.data, |s| {
                if drop_app.contains(xml::local(s).as_str()) { Action::Drop } else { Action::Keep }
            })?)
        } else if name == "docProps/custom.xml" && sel.contains("custom") {
            Some(xml::filter(&e.data, |s| if xml::local(s) == "property" { Action::Drop } else { Action::Keep })?)
        } else if name.ends_with(".rels") && (!removed.is_empty() || (rm_template && name == "word/_rels/settings.xml.rels")) {
            Some(xml::filter(&e.data, |s| {
                if xml::local(s) != "Relationship" {
                    return Action::Keep;
                }
                let target = xml::attr_value(s, "Target").unwrap_or_default();
                let ty = xml::attr_value(s, "Type").unwrap_or_default();
                if is_removed_target(&target) || (rm_template && ty.ends_with("/attachedTemplate")) {
                    Action::Drop
                } else {
                    Action::Keep
                }
            })?)
        } else if name == "[Content_Types].xml" && !removed.is_empty() {
            Some(xml::filter(&e.data, |s| {
                if xml::local(s) == "Override" {
                    let p = xml::attr_value(s, "PartName").unwrap_or_default();
                    if is_removed_target(&p) {
                        return Action::Drop;
                    }
                }
                Action::Keep
            })?)
        } else if name == "word/settings.xml" && rm_template {
            Some(xml::filter(&e.data, |s| if xml::local(s) == "attachedTemplate" { Action::Drop } else { Action::Keep })?)
        } else if rm_comments && name == "word/comments.xml" {
            Some(xml::filter(&e.data, |s| if xml::qname(s) == "w:comment" { Action::Drop } else { Action::Keep })?)
        } else if rm_comments && name.starts_with("word/comments") {
            Some(xml::filter(&e.data, |s| match xml::local(s).as_str() {
                "commentEx" | "commentId" | "commentExtensible" => Action::Drop,
                _ => Action::Keep,
            })?)
        } else if rm_comments && name == "word/people.xml" {
            Some(xml::filter(&e.data, |s| if xml::local(s) == "person" { Action::Drop } else { Action::Keep })?)
        } else if is_word_body(&name) && (rm_comments || rm_revisions) {
            Some(xml::filter(&e.data, |s| {
                let q = xml::qname(s);
                let Some(l) = q.strip_prefix("w:") else { return Action::Keep };
                if rm_comments && COMMENT_MARKERS.contains(&l) {
                    return Action::Drop;
                }
                if rm_revisions {
                    if REVISION_DROP.contains(&l) {
                        return Action::Drop;
                    }
                    if l == "ins" || l == "moveTo" {
                        return Action::Unwrap;
                    }
                }
                Action::Keep
            })?)
        } else {
            None
        };
        if let Some(n) = new {
            e.data = n;
        }
    }
    zipdoc::clean_media(&mut entries, sel, &mut notes, &["word/media/", "xl/media/", "ppt/media/"]);
    if rm_revisions {
        notes.push("Modifications suivies acceptées.".into());
    }
    if rm_comments {
        notes.push("Commentaires supprimés.".into());
    }
    Ok((zipdoc::write(&entries)?, notes))
}
