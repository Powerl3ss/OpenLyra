//! Lecture des paquets XMP (Adobe) pour en afficher le contenu.

use crate::model::{Entry, Risk};
use crate::xml;
use std::collections::BTreeMap;

fn friendly(key: &str) -> String {
    let k = key.to_string();
    let label = match xml::local_of(key) {
        "creator" => "Auteur",
        "CreatorTool" => "Logiciel de création",
        "CreateDate" => "Date de création",
        "ModifyDate" => "Date de modification",
        "MetadataDate" => "Date des métadonnées",
        "title" => "Titre",
        "description" => "Description",
        "rights" => "Droits",
        "subject" => "Mots-clés",
        "Producer" => "Producteur",
        "DocumentID" => "Identifiant du document",
        "InstanceID" => "Identifiant de version",
        "OriginalDocumentID" => "Identifiant du document d'origine",
        "GPSLatitude" => "Latitude GPS",
        "GPSLongitude" => "Longitude GPS",
        "City" => "Ville",
        "Country" => "Pays",
        "State" => "Région",
        "Make" => "Fabricant de l'appareil",
        "Model" => "Modèle de l'appareil",
        "SerialNumber" => "Numéro de série",
        "Lens" => "Objectif",
        "softwareAgent" => "Logiciel (historique)",
        "when" => "Date (historique)",
        "action" => "Action (historique)",
        _ => "",
    };
    if label.is_empty() {
        k
    } else {
        format!("{} ({})", label, k)
    }
}

/// Extrait les propriétés d'un paquet XMP.
pub fn entries(packet: &[u8]) -> Vec<Entry> {
    let tree = match xml::parse(packet) {
        Ok(t) => t,
        Err(_) => return vec![Entry::new("Paquet XMP", format!("{} octets (illisible)", packet.len()))],
    };
    let mut map: BTreeMap<String, Vec<String>> = BTreeMap::new();
    tree.walk(&mut |n, ancestors| {
        if n.local() == "Description" {
            for (k, v) in &n.attrs {
                if k.starts_with("xmlns") || k.starts_with("rdf:") || v.trim().is_empty() {
                    continue;
                }
                map.entry(k.clone()).or_default().push(v.clone());
            }
        }
        let text = n.text.trim();
        if !text.is_empty() && n.children.is_empty() {
            let mut key = None;
            for a in std::iter::once(n).chain(ancestors.iter().rev().copied()) {
                if !a.name.starts_with("rdf:") && !a.name.starts_with("x:") && !a.name.starts_with('#') {
                    key = Some(a.name.clone());
                    break;
                }
            }
            if let Some(k) = key {
                map.entry(k).or_default().push(text.to_string());
            }
        }
    });
    let mut out: Vec<Entry> = map
        .into_iter()
        .map(|(k, mut v)| {
            v.dedup();
            let more = if v.len() > 6 { format!(" (+{} autres)", v.len() - 6) } else { String::new() };
            v.truncate(6);
            Entry::new(friendly(&k), v.join(" | ") + &more)
        })
        .collect();
    if out.is_empty() {
        out.push(Entry::new("Paquet XMP", format!("{} octets, sans propriété lisible", packet.len())));
    }
    out.truncate(150);
    out
}

pub fn risk_of(entries: &[Entry]) -> Risk {
    let sensitive = ["creator", "GPS", "Auteur", "SerialNumber", "City", "rights", "Owner"];
    if entries.iter().any(|e| sensitive.iter().any(|s| e.key.contains(s))) {
        Risk::High
    } else {
        Risk::Medium
    }
}
