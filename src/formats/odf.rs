//! OpenDocument (LibreOffice : odt, ods, odp, odg…).

use super::{traces, zipdoc};
use crate::model::{Entry, FileKind, Group};
use crate::xml::{self, Action};
use anyhow::Result;
use std::collections::{BTreeMap, HashSet};

const META: &[(&str, &str, &str)] = &[
    ("meta:initial-creator", "people", "Créateur initial"),
    ("dc:creator", "people", "Modifié en dernier par"),
    ("meta:printed-by", "people", "Imprimé par"),
    ("meta:creation-date", "dates", "Date de création"),
    ("dc:date", "dates", "Date de dernière modification"),
    ("meta:print-date", "dates", "Dernière impression"),
    ("dc:title", "desc", "Titre"),
    ("dc:subject", "desc", "Sujet"),
    ("dc:description", "desc", "Commentaires"),
    ("meta:keyword", "desc", "Mot-clé"),
    ("dc:language", "desc", "Langue"),
    ("meta:generator", "software", "Application"),
    ("meta:template", "software", "Modèle"),
    ("meta:editing-cycles", "history", "Nombre d'enregistrements"),
    ("meta:editing-duration", "history", "Durée totale d'édition"),
    ("meta:document-statistic", "stats", "Statistiques"),
    ("meta:user-defined", "custom", ""),
];

const PRINTER_ITEMS: &[&str] = &["PrinterName", "PrinterSetup", "PrinterPaperFromSetup"];
const TRACK_DROP: &[&str] = &["text:tracked-changes", "text:change-start", "text:change-end", "text:change"];

pub fn kind_of(bytes: &[u8]) -> FileKind {
    let Ok(entries) = zipdoc::read(bytes) else { return FileKind::OdfOther };
    let mime = zipdoc::find(&entries, "mimetype").map(|e| String::from_utf8_lossy(&e.data).into_owned()).unwrap_or_default();
    if mime.contains("opendocument.text") {
        FileKind::OdfText
    } else if mime.contains("opendocument.spreadsheet") {
        FileKind::OdfSheet
    } else if mime.contains("opendocument.presentation") {
        FileKind::OdfPresentation
    } else {
        FileKind::OdfOther
    }
}

/// PT1H20M5S -> 1 h 20 min 5 s
fn pretty_duration(s: &str) -> String {
    let t = s.trim_start_matches('P');
    let mut out = Vec::new();
    let mut num = String::new();
    let mut in_time = false;
    for c in t.chars() {
        match c {
            'T' => in_time = true,
            '0'..='9' | '.' => num.push(c),
            'D' => { out.push(format!("{} j", num)); num.clear(); }
            'H' => { out.push(format!("{} h", num)); num.clear(); }
            'M' if in_time => { out.push(format!("{} min", num)); num.clear(); }
            'S' => { out.push(format!("{} s", num.split('.').next().unwrap_or(""))); num.clear(); }
            _ => num.clear(),
        }
    }
    if out.is_empty() { s.to_string() } else { out.join(" ") }
}

pub fn analyze(bytes: &[u8]) -> Result<(Vec<Group>, Vec<String>)> {
    let entries = zipdoc::read(bytes)?;
    let mut map: BTreeMap<String, Vec<Entry>> = BTreeMap::new();
    let mut risks = Vec::new();
    let mut push = |g: &str, e: Entry| map.entry(g.to_string()).or_default().push(e);

    if let Some(e) = zipdoc::find(&entries, "meta.xml") {
        let t = xml::parse(&e.data)?;
        for m in t.find_all("meta") {
            for c in &m.children {
                let Some((_, g, label)) = META.iter().find(|(n, _, _)| *n == c.name) else { continue };
                match *g {
                    "stats" => {
                        for (k, v) in &c.attrs {
                            push("stats", Entry::new(xml::local_of(k).replace('-', " "), v.clone()));
                        }
                    }
                    "custom" => push("custom", Entry::new(c.attr("name").unwrap_or("?"), c.all_text())),
                    _ => {
                        let mut v = c.all_text();
                        if c.name == "meta:template" {
                            v = c.attr("href").or(c.attr("title")).unwrap_or("").to_string();
                        }
                        if v.is_empty() {
                            continue;
                        }
                        if *g == "dates" {
                            v = xml::pretty_iso_date(&v);
                        }
                        if c.name == "meta:editing-duration" {
                            v = pretty_duration(&v);
                        }
                        push(g, Entry::new(*label, v));
                    }
                }
            }
        }
    }

    if let Some(e) = zipdoc::find(&entries, "content.xml") {
        if let Ok(t) = xml::parse(&e.data) {
            for a in t.find_all("annotation") {
                if a.name != "office:annotation" {
                    continue;
                }
                let author = a.children.iter().find(|c| c.name == "dc:creator").map(|c| c.all_text()).unwrap_or_else(|| "Inconnu".into());
                let date = a.children.iter().find(|c| c.name == "dc:date").map(|c| xml::pretty_iso_date(&c.all_text())).unwrap_or_default();
                let text: Vec<String> = a.children.iter().filter(|c| c.local() == "p").map(|c| c.all_text()).collect();
                let key = if date.is_empty() { author } else { format!("{} ({})", author, date) };
                push("comments", Entry::new(key, text.join(" ")));
            }
            let mut authors: BTreeMap<String, usize> = BTreeMap::new();
            for r in t.find_all("changed-region") {
                let who = r.find_all("creator").first().map(|c| c.all_text()).unwrap_or_else(|| "Inconnu".into());
                *authors.entry(who).or_default() += 1;
            }
            for (a, n) in authors {
                push("revisions", Entry::new(a, format!("{} modification(s) suivie(s)", n)));
            }
        }
    }

    if let Some(e) = zipdoc::find(&entries, "settings.xml") {
        if let Ok(t) = xml::parse(&e.data) {
            for c in t.find_all("config-item") {
                if c.attr("name") == Some("PrinterName") && !c.all_text().is_empty() {
                    push("printer", Entry::new("Imprimante", c.all_text()));
                }
            }
        }
    }

    for e in &entries {
        if e.name.starts_with("Thumbnails/") && !e.is_dir {
            push("thumb", Entry::new(e.name.clone(), crate::model::human_size(e.data.len() as u64)));
        }
        if e.name.starts_with("Pictures/") && zipdoc::is_image(&e.name) {
            if let Some((entry, r)) = zipdoc::media_entry(&e.name, &e.data) {
                push("media", entry);
                if r == crate::model::Risk::High {
                    risks.retain(|(i, _): &(String, _)| i != "media");
                    risks.push(("media".to_string(), r));
                }
            }
        }
    }
    let mut groups = zipdoc::assemble(map, &risks);
    for g in groups.iter_mut() {
        if g.id == "revisions" && kind_of(bytes) != FileKind::OdfText {
            *g = g.clone().info_only().note("Accepte les modifications depuis LibreOffice (Édition > Suivi des modifications).");
        }
    }
    groups.extend(traces::analyze_odf(&entries));
    groups.sort_by(|a, b| b.risk.cmp(&a.risk));
    Ok((groups, vec![]))
}

pub fn clean(bytes: &[u8], sel: &HashSet<String>) -> Result<(Vec<u8>, Vec<String>)> {
    let mut entries = zipdoc::read(bytes)?;
    let is_text = kind_of(bytes) == FileKind::OdfText;
    let mut notes = vec!["Dates internes de l'archive remises à zéro.".to_string()];
    let drop_meta: HashSet<&str> = META.iter().filter(|(_, g, _)| sel.contains(*g)).map(|(n, _, _)| *n).collect();
    let rm_thumb = sel.contains("thumb");
    let rm_comments = sel.contains("comments");
    let rm_rev = sel.contains("revisions") && is_text;
    let rm_printer = sel.contains("printer");

    if rm_thumb {
        entries.retain(|e| !e.name.starts_with("Thumbnails/"));
    }
    for e in entries.iter_mut() {
        let new = match e.name.as_str() {
            "meta.xml" if !drop_meta.is_empty() => Some(xml::filter(&e.data, |s| {
                if drop_meta.contains(xml::qname(s).as_str()) { Action::Drop } else { Action::Keep }
            })?),
            "META-INF/manifest.xml" if rm_thumb => Some(xml::filter(&e.data, |s| {
                if xml::local(s) == "file-entry"
                    && xml::attr_value(s, "full-path").map(|p| p.starts_with("Thumbnails/")).unwrap_or(false)
                {
                    Action::Drop
                } else {
                    Action::Keep
                }
            })?),
            "content.xml" | "styles.xml" if rm_comments || rm_rev => Some(xml::filter(&e.data, |s| {
                let q = xml::qname(s);
                if rm_comments && (q == "office:annotation" || q == "office:annotation-end") {
                    Action::Drop
                } else if rm_rev && TRACK_DROP.contains(&q.as_str()) {
                    Action::Drop
                } else {
                    Action::Keep
                }
            })?),
            "settings.xml" if rm_printer => Some(xml::filter(&e.data, |s| {
                if xml::local(s) == "config-item"
                    && xml::attr_value(s, "name").map(|n| PRINTER_ITEMS.contains(&n.as_str())).unwrap_or(false)
                {
                    Action::Drop
                } else {
                    Action::Keep
                }
            })?),
            _ => None,
        };
        if let Some(n) = new {
            e.data = n;
        }
    }
    zipdoc::clean_media(&mut entries, sel, &mut notes, &["Pictures/"]);
    traces::clean_odf(&mut entries, sel, &mut notes)?;
    if rm_rev {
        notes.push("Modifications suivies acceptées.".into());
    }
    Ok((zipdoc::write(&entries)?, notes))
}
