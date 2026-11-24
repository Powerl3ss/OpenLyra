//! Traces dans le CONTENU des documents Word et LibreOffice (au-delà des
//! propriétés) : identifiants de session, chemins locaux, champs liés à
//! l'auteur, variables cachées, texte masqué, signets.

use super::zipdoc::ZipEntry;
use crate::model::{Entry, Group, Risk};
use crate::xml::{self, Action, AttrAction, Node};
use anyhow::Result;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

// ---------------------------------------------------------------------------
// Définitions communes
// ---------------------------------------------------------------------------

pub fn group(id: &str) -> Group {
    match id {
        "trace.paths" => Group::new(
            id,
            "Chemins de fichiers locaux",
            "Emplacements de fichiers sur l'ordinateur d'origine (liens, images liées, publipostage). Ils contiennent souvent le nom d'utilisateur Windows.",
            Risk::High,
        )
        .note("Les liens vers ces fichiers ne garderont que le nom du fichier, sans le dossier."),
        "trace.fields" => Group::new(
            id,
            "Champs liés à l'auteur",
            "Champs automatiques qui affichent ou mémorisent le nom de l'auteur, ses initiales, ses coordonnées ou le chemin du fichier.",
            Risk::High,
        )
        .note("Ces champs et le texte qu'ils affichent sont retirés du document."),
        "trace.rsid" => Group::new(
            id,
            "Identifiants de session d'édition",
            "Codes ajoutés à chaque session de travail. Ils permettent de relier ce document à d'autres documents édités sur le même ordinateur ou aux mêmes moments.",
            Risk::Medium,
        ),
        "trace.vars" => Group::new(
            id,
            "Variables cachées",
            "Valeurs stockées par des macros, des modèles ou des logiciels, invisibles à l'écran.",
            Risk::Medium,
        ),
        "trace.settings" => Group::new(
            id,
            "Paramètres enregistrés",
            "Source de données de publipostage et télécopieur mémorisés dans le document.",
            Risk::Medium,
        ),
        "trace.hidden" => Group::new(
            id,
            "Texte masqué",
            "Texte présent dans le fichier mais invisible à l'écran et à l'impression.",
            Risk::High,
        )
        .not_recommended()
        .note("Supprime définitivement ce texte du document."),
        "trace.bookmarks" => Group::new(
            id,
            "Signets",
            "Noms internes des signets. Ils peuvent trahir l'outil qui a produit le document ou garder la dernière position d'édition.",
            Risk::Low,
        )
        .not_recommended()
        .note("Les signets sont renommés de façon neutre ; les renvois et la table des matières continuent de fonctionner."),
        _ => Group::new(id, id, "", Risk::Medium),
    }
}

const ORDER: &[&str] = &[
    "trace.paths",
    "trace.fields",
    "trace.hidden",
    "trace.rsid",
    "trace.vars",
    "trace.settings",
    "trace.bookmarks",
];

fn assemble(map: BTreeMap<&'static str, Vec<Entry>>) -> Vec<Group> {
    ORDER
        .iter()
        .filter_map(|id| {
            let e = map.get(id)?;
            if e.is_empty() {
                None
            } else {
                Some(group(id).entries(e.clone()))
            }
        })
        .collect()
}

/// Chemin pointant vers l'ordinateur de l'auteur ?
pub fn is_local_path(s: &str) -> bool {
    let t = s.trim().trim_matches('"');
    let l = t.to_ascii_lowercase();
    let b = t.as_bytes();
    l.starts_with("file:")
        || (b.len() > 2 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/'))
        || t.starts_with("\\\\")
        || ((t.starts_with("../") || t.starts_with("..\\")) && t[3..].contains(['/', '\\']))
        || l.contains("/users/")
        || l.contains("\\users\\")
        || l.starts_with("/home/")
}

/// Ne garde que le nom du fichier.
pub fn basename(s: &str) -> String {
    let t = s.trim();
    let quoted = t.starts_with('"') && t.ends_with('"') && t.len() >= 2;
    let inner = if quoted { &t[1..t.len() - 1] } else { t };
    let name = inner
        .rsplit(['/', '\\'])
        .find(|p| !p.is_empty())
        .unwrap_or(inner)
        .to_string();
    if quoted {
        format!("\"{name}\"")
    } else {
        name
    }
}

/// Remplace les chemins locaux présents dans une instruction de champ.
fn sanitize_instr(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(start) = rest.find('"') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('"') {
            Some(end) => {
                let inner = &after[..end];
                let v = if is_local_path(inner) { basename(inner) } else { inner.to_string() };
                out.push('"');
                out.push_str(&v);
                out.push('"');
                rest = &after[end + 1..];
            }
            None => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

fn local_paths_in(s: &str) -> Vec<String> {
    let mut v = Vec::new();
    let mut parts = s.split('"');
    parts.next();
    for (i, p) in parts.enumerate() {
        if i % 2 == 0 && is_local_path(p) {
            v.push(p.to_string());
        }
    }
    v
}

/// Remplace des identifiants entiers (noms de signets) dans un texte.
fn replace_names(s: &str, map: &HashMap<String, String>) -> String {
    if map.is_empty() {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut word = String::new();
    let is_id = |c: char| c.is_alphanumeric() || c == '_';
    for c in s.chars() {
        if is_id(c) {
            word.push(c);
        } else {
            if !word.is_empty() {
                out.push_str(map.get(&word).map(|x| x.as_str()).unwrap_or(&word));
                word.clear();
            }
            out.push(c);
        }
    }
    if !word.is_empty() {
        out.push_str(map.get(&word).map(|x| x.as_str()).unwrap_or(&word));
    }
    out
}

fn bookmark_kind(name: &str) -> &'static str {
    if name == "_GoBack" {
        "Dernière position d'édition (sera supprimé)"
    } else if name.starts_with("_Toc") || name.starts_with("__RefHeading") {
        "Table des matières"
    } else if name.starts_with("_Ref") {
        "Renvoi"
    } else if name.starts_with("_Hlk") {
        "Lien interne créé par un copier-coller"
    } else if name.starts_with('_') {
        "Signet masqué"
    } else {
        "Signet nommé"
    }
}

/// Signet déjà renommé par Lyra (ne compte plus comme une trace).
fn is_neutral_bookmark(name: &str) -> bool {
    let num = |s: &str| s.parse::<u64>().ok();
    if let Some(n) = name.strip_prefix("Signet").and_then(num) {
        return n > 0;
    }
    if let Some(n) = name.strip_prefix("__RefHeading___Toc").and_then(num) {
        return (100_001..=199_999).contains(&n);
    }
    for p in ["_Toc", "_Hlk", "_Ref"] {
        if let Some(rest) = name.strip_prefix(p) {
            if rest.len() == 9 {
                if let Some(n) = num(rest) {
                    return (100_000_001..=100_999_999).contains(&n);
                }
            }
        }
    }
    false
}

/// Nouveaux noms neutres, en gardant le préfixe dont dépend le logiciel.
fn bookmark_map(names: &BTreeSet<String>) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let mut n = 0u32;
    for name in names {
        if name == "_GoBack" {
            continue;
        }
        n += 1;
        let new = if name.starts_with("__RefHeading") {
            format!("__RefHeading___Toc{}", 100000 + n)
        } else if name.starts_with("_Toc") {
            format!("_Toc{}", 100000000 + n)
        } else if name.starts_with("_Hlk") {
            format!("_Hlk{}", 100000000 + n)
        } else if name.starts_with('_') {
            format!("_Ref{}", 100000000 + n)
        } else {
            format!("Signet{n}")
        };
        map.insert(name.clone(), new);
    }
    map
}

fn preview(s: &str) -> String {
    let t: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if t.chars().count() > 120 {
        t.chars().take(120).collect::<String>() + "…"
    } else {
        t
    }
}

// ---------------------------------------------------------------------------
// Word
// ---------------------------------------------------------------------------

fn word_field_label(instr: &str) -> Option<&'static str> {
    let up = instr.trim().to_uppercase();
    let first = up.split_whitespace().next().unwrap_or("");
    match first {
        "AUTHOR" => Some("Champ AUTEUR"),
        "USERNAME" => Some("Champ NOM D'UTILISATEUR"),
        "USERINITIALS" => Some("Champ INITIALES"),
        "USERADDRESS" => Some("Champ ADRESSE"),
        "LASTSAVEDBY" => Some("Champ ENREGISTRÉ PAR"),
        "FILENAME" if up.contains("\\P") => Some("Champ NOM DE FICHIER (avec chemin)"),
        "TEMPLATE" if up.contains("\\P") => Some("Champ MODÈLE (avec chemin)"),
        "DOCPROPERTY"
            if ["AUTHOR", "LASTAUTHOR", "LAST AUTHOR", "COMPANY", "MANAGER"].iter().any(|k| up.contains(k)) =>
        {
            Some("Champ PROPRIÉTÉ (auteur, société…)")
        }
        _ => None,
    }
}

fn is_word_xml(name: &str) -> bool {
    name.starts_with("word/") && name.ends_with(".xml")
}

fn is_word_story(name: &str) -> bool {
    name == "word/document.xml"
        || name.starts_with("word/header")
        || name.starts_with("word/footer")
        || name == "word/footnotes.xml"
        || name == "word/endnotes.xml"
        || name == "word/comments.xml"
}

/// Résultat de l'examen des « runs » (w:r) d'une partie, dans l'ordre du document.
#[derive(Default)]
struct RunScan {
    field_runs: HashSet<usize>,
    hidden_runs: HashSet<usize>,
    fields: Vec<Entry>,
    hidden: Vec<Entry>,
}

fn scan_runs(tree: &Node) -> RunScan {
    struct Field {
        start: usize,
        instr: String,
        result: String,
        in_result: bool,
    }
    let mut out = RunScan::default();
    let mut idx = 0usize;
    let mut stack: Vec<Field> = Vec::new();
    tree.walk(&mut |n, _| {
        if n.name == "w:fldSimple" {
            if let Some(instr) = n.attr("instr") {
                if let Some(label) = word_field_label(instr) {
                    out.fields.push(Entry::new(label, preview(&n.all_text())));
                }
            }
            return;
        }
        if n.name != "w:r" {
            return;
        }
        let i = idx;
        idx += 1;
        let mut text = String::new();
        let mut hidden = false;
        for c in &n.children {
            match c.name.as_str() {
                "w:fldChar" => match c.attr("fldCharType").unwrap_or("") {
                    "begin" => stack.push(Field { start: i, instr: String::new(), result: String::new(), in_result: false }),
                    "separate" => {
                        if let Some(f) = stack.last_mut() {
                            f.in_result = true;
                        }
                    }
                    "end" => {
                        if let Some(f) = stack.pop() {
                            if let Some(label) = word_field_label(&f.instr) {
                                out.field_runs.extend(f.start..=i);
                                let v = if f.result.trim().is_empty() { f.instr.trim().to_string() } else { f.result.clone() };
                                out.fields.push(Entry::new(label, preview(&v)));
                            }
                        }
                    }
                    _ => {}
                },
                "w:instrText" => {
                    if let Some(f) = stack.last_mut() {
                        f.instr.push_str(&c.text);
                    }
                }
                "w:t" => text.push_str(&c.text),
                "w:rPr" => {
                    let vanish = c.children.iter().any(|x| x.name == "w:vanish" && x.attr("val") != Some("0") && x.attr("val") != Some("false"));
                    let special = c.children.iter().any(|x| x.name == "w:specVanish");
                    hidden = vanish && !special;
                }
                _ => {}
            }
        }
        if let Some(f) = stack.last_mut() {
            if f.in_result {
                f.result.push_str(&text);
            }
        }
        if hidden && !text.trim().is_empty() {
            out.hidden_runs.insert(i);
            out.hidden.push(Entry::new(format!("Passage masqué {}", out.hidden.len() + 1), preview(&text)));
        }
    });
    // Un passage masqué déjà compris dans un champ supprimé n'est compté qu'une fois.
    out.hidden_runs.retain(|r| !out.field_runs.contains(r));
    out
}

fn part_label(name: &str) -> &str {
    if name == "word/document.xml" {
        "corps du texte"
    } else if name.starts_with("word/header") {
        "en-tête"
    } else if name.starts_with("word/footer") {
        "pied de page"
    } else if name.contains("footnotes") {
        "notes de bas de page"
    } else if name.contains("endnotes") {
        "notes de fin"
    } else if name.contains("comments") {
        "commentaires"
    } else {
        name
    }
}

pub fn analyze_word(entries: &[ZipEntry]) -> Vec<Group> {
    let mut map: BTreeMap<&'static str, Vec<Entry>> = BTreeMap::new();
    let mut rsid_values: BTreeSet<String> = BTreeSet::new();
    let mut rsid_marks = 0usize;
    let mut para_ids = 0usize;
    let mut bookmarks: BTreeSet<String> = BTreeSet::new();

    for e in entries {
        if e.name.ends_with(".rels") {
            if let Ok(t) = xml::parse(&e.data) {
                for r in t.find_all("Relationship") {
                    let target = r.attr("Target").unwrap_or("");
                    if r.attr("TargetMode") == Some("External") && is_local_path(target) {
                        map.entry("trace.paths").or_default().push(Entry::new("Lien externe", target));
                    }
                }
            }
            continue;
        }
        if !is_word_xml(&e.name) {
            continue;
        }
        let Ok(tree) = xml::parse(&e.data) else { continue };
        tree.walk(&mut |n, _| {
            for (k, v) in &n.attrs {
                if k.starts_with("w:rsid") {
                    rsid_marks += 1;
                    rsid_values.insert(v.clone());
                }
                if k == "w14:paraId" || k == "w14:textId" {
                    para_ids += 1;
                }
            }
            match n.name.as_str() {
                "w:rsid" | "w:rsidRoot" => {
                    if let Some(v) = n.attr("val") {
                        rsid_values.insert(v.to_string());
                    }
                }
                "w:docVar" => map.entry("trace.vars").or_default().push(Entry::new(
                    n.attr("name").unwrap_or("?"),
                    preview(n.attr("val").unwrap_or("")),
                )),
                "w:mailMerge" => {
                    let q = n.find_all("query").first().and_then(|q| q.attr("val")).unwrap_or("").to_string();
                    let src = n.find_all("connectString").first().and_then(|q| q.attr("val")).unwrap_or("").to_string();
                    let v = [q, src].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" | ");
                    map.entry("trace.paths").or_default().push(Entry::new("Source de publipostage", preview(&v)));
                }
                "w:instrText" => {
                    for p in local_paths_in(&n.text) {
                        map.entry("trace.paths").or_default().push(Entry::new("Chemin dans un champ", p.replace("\\\\", "\\")));
                    }
                }
                "w:fldSimple" => {
                    for p in local_paths_in(n.attr("instr").unwrap_or("")) {
                        map.entry("trace.paths").or_default().push(Entry::new("Chemin dans un champ", p.replace("\\\\", "\\")));
                    }
                }
                "w:bookmarkStart" => {
                    if let Some(name) = n.attr("name") {
                        bookmarks.insert(name.to_string());
                    }
                }
                _ => {}
            }
        });
        if is_word_story(&e.name) {
            let scan = scan_runs(&tree);
            let where_ = part_label(&e.name);
            for f in scan.fields {
                map.entry("trace.fields").or_default().push(Entry::new(format!("{} ({})", f.key, where_), f.value));
            }
            for h in scan.hidden {
                map.entry("trace.hidden").or_default().push(Entry::new(format!("{} ({})", h.key, where_), h.value));
            }
        }
    }
    if rsid_marks > 0 || !rsid_values.is_empty() {
        let v = map.entry("trace.rsid").or_default();
        v.push(Entry::new("Sessions d'édition distinctes", rsid_values.len().to_string()));
        v.push(Entry::new("Éléments marqués", rsid_marks.to_string()));
    }
    if para_ids > 0 {
        map.entry("trace.rsid")
            .or_default()
            .push(Entry::new("Identifiants de paragraphes", para_ids.to_string()));
    }
    bookmarks.retain(|b| !is_neutral_bookmark(b));
    if !bookmarks.is_empty() {
        let v = map.entry("trace.bookmarks").or_default();
        for b in bookmarks.iter().take(60) {
            v.push(Entry::new(b.clone(), bookmark_kind(b)));
        }
        if bookmarks.len() > 60 {
            v.push(Entry::new("…", format!("{} signets au total", bookmarks.len())));
        }
    }
    assemble(map)
}

pub fn clean_word(entries: &mut [ZipEntry], sel: &HashSet<String>, notes: &mut Vec<String>) -> Result<()> {
    let rm_paths = sel.contains("trace.paths");
    let rm_fields = sel.contains("trace.fields");
    let rm_rsid = sel.contains("trace.rsid");
    let rm_vars = sel.contains("trace.vars");
    let rm_hidden = sel.contains("trace.hidden");
    let rm_bm = sel.contains("trace.bookmarks");
    if !(rm_paths || rm_fields || rm_rsid || rm_vars || rm_hidden || rm_bm) {
        return Ok(());
    }
    // Les identifiants de paragraphes servent à relier les commentaires : on
    // les garde si des commentaires restent dans le document.
    let has_comment_links = entries.iter().any(|e| e.name == "word/commentsExtended.xml");
    let rm_paraid = rm_rsid && (!has_comment_links || sel.contains("comments"));
    if rm_rsid && !rm_paraid {
        notes.push("Identifiants de paragraphes conservés car ils relient les commentaires du document.".into());
    }

    // Signets : table de renommage et identifiants de « _GoBack ».
    let mut names: BTreeSet<String> = BTreeSet::new();
    let mut goback_ids: HashSet<String> = HashSet::new();
    if rm_bm {
        for e in entries.iter().filter(|e| is_word_xml(&e.name)) {
            if let Ok(t) = xml::parse(&e.data) {
                for b in t.find_all("bookmarkStart") {
                    if let Some(n) = b.attr("name") {
                        names.insert(n.to_string());
                        if n == "_GoBack" {
                            if let Some(id) = b.attr("id") {
                                goback_ids.insert(id.to_string());
                            }
                        }
                    }
                }
            }
        }
    }
    let renames = bookmark_map(&names);
    let (mut n_fields, mut n_hidden) = (0, 0);

    for e in entries.iter_mut() {
        let is_rels = e.name.ends_with(".rels");
        if !(is_word_xml(&e.name) || (is_rels && rm_paths)) {
            continue;
        }
        let drop_runs: HashSet<usize> = if is_word_story(&e.name) && (rm_fields || rm_hidden) {
            let tree = xml::parse(&e.data)?;
            let scan = scan_runs(&tree);
            let mut d = HashSet::new();
            if rm_fields {
                n_fields += scan.fields.len();
                d.extend(scan.field_runs);
            }
            if rm_hidden {
                n_hidden += scan.hidden_runs.len();
                d.extend(scan.hidden_runs);
            }
            d
        } else {
            HashSet::new()
        };
        let mut run_idx = 0usize;
        e.data = xml::transform(
            &e.data,
            |s, skipping| {
                let q = xml::qname(s);
                if q == "w:r" {
                    let i = run_idx;
                    run_idx += 1;
                    if !skipping && drop_runs.contains(&i) {
                        return Action::Drop;
                    }
                }
                if skipping {
                    return Action::Keep;
                }
                match q.as_str() {
                    "w:rsids" | "w:rsid" | "w:rsidRoot" if rm_rsid => Action::Drop,
                    "w:docVars" if rm_vars => Action::Drop,
                    "w:mailMerge" if rm_paths => Action::Drop,
                    "w:fldSimple" if rm_fields => {
                        let instr = xml::attr_value(s, "instr").unwrap_or_default();
                        if word_field_label(&instr).is_some() {
                            Action::Drop
                        } else {
                            Action::Keep
                        }
                    }
                    "w:bookmarkStart" if rm_bm && xml::attr_value(s, "name").as_deref() == Some("_GoBack") => Action::Drop,
                    "w:bookmarkEnd" if rm_bm && goback_ids.contains(&xml::attr_value(s, "id").unwrap_or_default()) => {
                        Action::Drop
                    }
                    _ => Action::Keep,
                }
            },
            |el, k, v| {
                if rm_rsid && k.starts_with("w:rsid") {
                    return AttrAction::Drop;
                }
                if rm_paraid && (k == "w14:paraId" || k == "w14:textId") {
                    return AttrAction::Drop;
                }
                if rm_paths && el == "Relationship" && k == "Target" && is_local_path(v) {
                    return AttrAction::Set(basename(v));
                }
                if rm_bm && ((el == "w:bookmarkStart" && k == "w:name") || k == "w:anchor") {
                    if let Some(n) = renames.get(v) {
                        return AttrAction::Set(n.clone());
                    }
                }
                if el == "w:fldSimple" && k == "w:instr" {
                    let mut n = v.to_string();
                    if rm_paths {
                        n = sanitize_instr(&n);
                    }
                    if rm_bm {
                        n = replace_names(&n, &renames);
                    }
                    return AttrAction::Set(n);
                }
                AttrAction::Keep
            },
            |parent, t| {
                if parent != "w:instrText" {
                    return None;
                }
                let mut n = t.to_string();
                if rm_paths {
                    n = sanitize_instr(&n);
                }
                if rm_bm {
                    n = replace_names(&n, &renames);
                }
                Some(n)
            },
        )?;
    }
    if rm_fields && n_fields > 0 {
        notes.push(format!("{n_fields} champ(s) lié(s) à l'auteur retiré(s)."));
    }
    if rm_hidden && n_hidden > 0 {
        notes.push(format!("{n_hidden} passage(s) de texte masqué supprimé(s)."));
    }
    if rm_bm && !names.is_empty() {
        notes.push(format!("{} signet(s) renommé(s) ou supprimé(s).", names.len()));
    }
    Ok(())
}

/// Chemins locaux dans les liens d'Excel et PowerPoint (fichiers .rels).
pub fn analyze_rels_only(entries: &[ZipEntry]) -> Vec<Group> {
    let mut v = Vec::new();
    for e in entries.iter().filter(|e| e.name.ends_with(".rels")) {
        if let Ok(t) = xml::parse(&e.data) {
            for r in t.find_all("Relationship") {
                let target = r.attr("Target").unwrap_or("");
                if r.attr("TargetMode") == Some("External") && is_local_path(target) {
                    v.push(Entry::new("Lien externe", target));
                }
            }
        }
    }
    if v.is_empty() {
        vec![]
    } else {
        vec![group("trace.paths").entries(v)]
    }
}

pub fn clean_rels_only(entries: &mut [ZipEntry], sel: &HashSet<String>) -> Result<()> {
    if !sel.contains("trace.paths") {
        return Ok(());
    }
    for e in entries.iter_mut().filter(|e| e.name.ends_with(".rels")) {
        e.data = xml::transform(
            &e.data,
            |_, _| Action::Keep,
            |el, k, v| {
                if el == "Relationship" && k == "Target" && is_local_path(v) {
                    AttrAction::Set(basename(v))
                } else {
                    AttrAction::Keep
                }
            },
            |_, _| None,
        )?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// LibreOffice (OpenDocument)
// ---------------------------------------------------------------------------

const ODF_PARTS: &[&str] = &["content.xml", "styles.xml"];
const ODF_SETTINGS: &[&str] = &[
    "CurrentDatabaseDataSource",
    "CurrentDatabaseCommand",
    "CurrentDatabaseCommandType",
    "PrintFaxName",
];
const ODF_RSID_SETTINGS: &[&str] = &["Rsid", "RsidRoot"];

fn odf_field_label(n: &Node) -> Option<&'static str> {
    let q = n.name.as_str();
    match q {
        "text:author-name" => Some("Champ Auteur"),
        "text:author-initials" => Some("Champ Initiales de l'auteur"),
        "text:initial-creator" => Some("Champ Créateur du document"),
        "text:creator" => Some("Champ Modifié par"),
        "text:file-name" | "text:template-name"
            if matches!(n.attr("display"), Some("full") | Some("path")) =>
        {
            Some("Champ Chemin du fichier")
        }
        _ if q.starts_with("text:sender-") => Some("Champ Données de l'expéditeur"),
        _ => None,
    }
}

fn odf_field_qname_removable(q: &str, display: Option<&str>) -> bool {
    matches!(q, "text:author-name" | "text:author-initials" | "text:initial-creator" | "text:creator")
        || q.starts_with("text:sender-")
        || (matches!(q, "text:file-name" | "text:template-name") && matches!(display, Some("full") | Some("path")))
}

fn odf_hidden(q: &str, display: Option<&str>) -> bool {
    matches!(q, "text:hidden-text" | "text:hidden-paragraph") || (q == "text:section" && display == Some("none"))
}

pub fn analyze_odf(entries: &[ZipEntry]) -> Vec<Group> {
    let mut map: BTreeMap<&'static str, Vec<Entry>> = BTreeMap::new();
    let mut rsids: BTreeSet<String> = BTreeSet::new();
    let mut rsid_marks = 0usize;
    let mut bookmarks: BTreeSet<String> = BTreeSet::new();

    for e in entries {
        let name = e.name.as_str();
        if name == "settings.xml" {
            if let Ok(t) = xml::parse(&e.data) {
                for c in t.find_all("config-item") {
                    let n = c.attr("name").unwrap_or("");
                    let v = c.all_text();
                    if ODF_SETTINGS.contains(&n) && !v.is_empty() && v != "0" {
                        map.entry("trace.settings").or_default().push(Entry::new(n, preview(&v)));
                    }
                    if ODF_RSID_SETTINGS.contains(&n) && !v.is_empty() {
                        rsids.insert(v);
                    }
                }
            }
            continue;
        }
        if !ODF_PARTS.contains(&name) {
            continue;
        }
        let Ok(t) = xml::parse(&e.data) else { continue };
        t.walk(&mut |n, _| {
            for (k, v) in &n.attrs {
                if k == "officeooo:rsid" || k == "officeooo:paragraph-rsid" {
                    rsid_marks += 1;
                    rsids.insert(v.clone());
                }
                if k == "xlink:href" && is_local_path(v) {
                    map.entry("trace.paths").or_default().push(Entry::new("Lien vers un fichier local", v.clone()));
                }
            }
            if n.name == "text:user-field-decl" || n.name == "text:variable-decl" {
                let v = n.attr("string-value").or(n.attr("value")).unwrap_or("");
                if !v.trim().is_empty() {
                    map.entry("trace.vars").or_default().push(Entry::new(n.attr("name").unwrap_or("?"), preview(v)));
                }
            }
            if let Some(label) = odf_field_label(n) {
                map.entry("trace.fields").or_default().push(Entry::new(label, preview(&n.all_text())));
            }
            let display = n.attr("display");
            if odf_hidden(&n.name, display) {
                let mut txt = n.attr("string-value").map(|s| s.to_string()).unwrap_or_else(|| n.all_text());
                let label = match n.name.as_str() {
                    "text:section" => "Section masquée",
                    "text:hidden-paragraph" => "Paragraphe masqué",
                    _ => "Texte masqué",
                };
                if txt.trim().is_empty() {
                    txt = "(sans texte visible, masqué par une condition)".into();
                }
                map.entry("trace.hidden").or_default().push(Entry::new(label, preview(&txt)));
            }
            if matches!(n.name.as_str(), "text:bookmark" | "text:bookmark-start") {
                if let Some(b) = n.attr("name") {
                    bookmarks.insert(b.to_string());
                }
            }
        });
    }
    if rsid_marks > 0 || !rsids.is_empty() {
        let v = map.entry("trace.rsid").or_default();
        v.push(Entry::new("Sessions d'édition distinctes", rsids.len().to_string()));
        if rsid_marks > 0 {
            v.push(Entry::new("Éléments marqués", rsid_marks.to_string()));
        }
    }
    bookmarks.retain(|b| !is_neutral_bookmark(b));
    if !bookmarks.is_empty() {
        let v = map.entry("trace.bookmarks").or_default();
        for b in bookmarks.iter().take(60) {
            v.push(Entry::new(b.clone(), bookmark_kind(b)));
        }
    }
    assemble(map)
}

pub fn clean_odf(entries: &mut [ZipEntry], sel: &HashSet<String>, notes: &mut Vec<String>) -> Result<()> {
    let rm_paths = sel.contains("trace.paths");
    let rm_fields = sel.contains("trace.fields");
    let rm_rsid = sel.contains("trace.rsid");
    let rm_settings = sel.contains("trace.settings");
    let rm_hidden = sel.contains("trace.hidden");
    let rm_bm = sel.contains("trace.bookmarks");
    let rm_vars = sel.contains("trace.vars");
    if !(rm_paths || rm_fields || rm_rsid || rm_settings || rm_hidden || rm_bm || rm_vars) {
        return Ok(());
    }
    let mut names: BTreeSet<String> = BTreeSet::new();
    if rm_bm {
        for e in entries.iter().filter(|e| ODF_PARTS.contains(&e.name.as_str())) {
            if let Ok(t) = xml::parse(&e.data) {
                t.walk(&mut |n, _| {
                    if matches!(n.name.as_str(), "text:bookmark" | "text:bookmark-start") {
                        if let Some(b) = n.attr("name") {
                            names.insert(b.to_string());
                        }
                    }
                });
            }
        }
    }
    let renames = bookmark_map(&names);
    let (mut n_fields, mut n_hidden) = (0usize, 0usize);

    for e in entries.iter_mut() {
        let name = e.name.clone();
        if name == "settings.xml" && (rm_settings || rm_rsid) {
            e.data = xml::filter(&e.data, |s| {
                if xml::local(s) == "config-item" {
                    let n = xml::attr_value(s, "name").unwrap_or_default();
                    if (rm_settings && ODF_SETTINGS.contains(&n.as_str()))
                        || (rm_rsid && ODF_RSID_SETTINGS.contains(&n.as_str()))
                    {
                        return Action::Drop;
                    }
                }
                Action::Keep
            })?;
            continue;
        }
        if !ODF_PARTS.contains(&name.as_str()) {
            continue;
        }
        e.data = xml::transform(
            &e.data,
            |s, skipping| {
                if skipping {
                    return Action::Keep;
                }
                let q = xml::qname(s);
                let display = xml::attr_value(s, "display");
                if rm_fields && odf_field_qname_removable(&q, display.as_deref()) {
                    n_fields += 1;
                    return Action::Drop;
                }
                if rm_hidden && odf_hidden(&q, display.as_deref()) {
                    n_hidden += 1;
                    return Action::Drop;
                }
                if rm_bm
                    && matches!(q.as_str(), "text:bookmark" | "text:bookmark-start" | "text:bookmark-end")
                    && xml::attr_value(s, "name").as_deref() == Some("_GoBack")
                {
                    return Action::Drop;
                }
                Action::Keep
            },
            |el, k, v| {
                if rm_rsid && (k == "officeooo:rsid" || k == "officeooo:paragraph-rsid") {
                    return AttrAction::Drop;
                }
                if rm_vars
                    && (el == "text:user-field-decl" || el == "text:variable-decl")
                    && (k == "office:string-value" || k == "office:value")
                {
                    return AttrAction::Set(if k == "office:value" { "0".into() } else { String::new() });
                }
                if k == "xlink:href" {
                    if rm_bm {
                        if let Some(anchor) = v.strip_prefix('#') {
                            let (target, rest) = match anchor.find('|') {
                                Some(p) => (&anchor[..p], &anchor[p..]),
                                None => (anchor, ""),
                            };
                            if let Some(n) = renames.get(target) {
                                return AttrAction::Set(format!("#{n}{rest}"));
                            }
                        }
                    }
                    if rm_paths && is_local_path(v) {
                        // « ../nom » = fichier situé à côté du document. Un nom seul
                        // serait cherché à l'intérieur du fichier et l'empêcherait de s'ouvrir.
                        return AttrAction::Set(format!("../{}", basename(v)));
                    }
                }
                if rm_bm
                    && ((matches!(el, "text:bookmark" | "text:bookmark-start" | "text:bookmark-end") && k == "text:name")
                        || (el == "text:bookmark-ref" && k == "text:ref-name"))
                {
                    if let Some(n) = renames.get(v) {
                        return AttrAction::Set(n.clone());
                    }
                }
                AttrAction::Keep
            },
            |_, _| None,
        )?;
    }
    if rm_fields && n_fields > 0 {
        notes.push(format!("{n_fields} champ(s) lié(s) à l'auteur retiré(s)."));
    }
    if rm_hidden && n_hidden > 0 {
        notes.push(format!("{n_hidden} élément(s) masqué(s) supprimé(s)."));
    }
    if rm_bm && !names.is_empty() {
        notes.push(format!("{} signet(s) renommé(s).", names.len()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths() {
        assert!(is_local_path("file:///C:/Users/Jean/Desktop/a.docx"));
        assert!(is_local_path("C:\\Users\\Jean\\x.png"));
        assert!(is_local_path("\\\\serveur\\partage\\x.xlsx"));
        assert!(is_local_path("../../Documents/image.png"));
        assert!(!is_local_path("../image.png"));
        assert!(!is_local_path("https://example.org/a"));
        assert!(!is_local_path("media/image1.png"));
        assert_eq!(basename("C:\\Users\\Jean\\x.png"), "x.png");
        assert_eq!(basename("file:///home/jean/a%20b.docx"), "a%20b.docx");
        assert_eq!(
            sanitize_instr(r#" INCLUDEPICTURE "C:\\Users\\Jean\\photo.jpg" \* MERGEFORMATINET "#),
            r#" INCLUDEPICTURE "photo.jpg" \* MERGEFORMATINET "#
        );
    }

    #[test]
    fn bookmarks() {
        let names: BTreeSet<String> = ["_Toc12345", "_GoBack", "intro", "_Ref99"].iter().map(|s| s.to_string()).collect();
        let m = bookmark_map(&names);
        assert!(!m.contains_key("_GoBack"));
        assert!(m["_Toc12345"].starts_with("_Toc"));
        assert!(m["_Ref99"].starts_with("_Ref"));
        assert!(m["intro"].starts_with("Signet"));
        let s = replace_names(" PAGEREF _Toc12345 \\h ", &m);
        assert_eq!(s, format!(" PAGEREF {} \\h ", m["_Toc12345"]));
        assert_eq!(replace_names("REF intro_bis", &m), "REF intro_bis");
    }

    #[test]
    fn neutral() {
        assert!(is_neutral_bookmark("Signet3"));
        assert!(is_neutral_bookmark("_Toc100000002"));
        assert!(is_neutral_bookmark("__RefHeading___Toc100004"));
        assert!(!is_neutral_bookmark("_Toc184000001"));
        assert!(!is_neutral_bookmark("_GoBack"));
        assert!(!is_neutral_bookmark("intro"));
    }

    #[test]
    fn fields() {
        assert!(word_field_label(" AUTHOR \\* MERGEFORMAT ").is_some());
        assert!(word_field_label(" FILENAME \\p ").is_some());
        assert!(word_field_label(" FILENAME ").is_none());
        assert!(word_field_label(" PAGE ").is_none());
    }
}
