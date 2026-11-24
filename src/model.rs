//! Modèle de données commun : un rapport contient des groupes de métadonnées.

use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Risk {
    Low,
    Medium,
    High,
}

impl Risk {
    pub fn label(&self) -> &'static str {
        match self {
            Risk::Low => "Faible",
            Risk::Medium => "Moyen",
            Risk::High => "Élevé",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub key: String,
    pub value: String,
    pub link: Option<String>,
}

impl Entry {
    pub fn new(key: impl Into<String>, value: impl Into<String>) -> Self {
        let mut value: String = value.into();
        value = value.trim().to_string();
        if value.chars().count() > 400 {
            value = value.chars().take(400).collect::<String>() + "…";
        }
        Entry { key: key.into(), value, link: None }
    }

    pub fn with_link(mut self, link: impl Into<String>) -> Self {
        self.link = Some(link.into());
        self
    }
}

/// Un groupe de métadonnées que l'utilisateur peut choisir de supprimer.
#[derive(Clone, Debug)]
pub struct Group {
    pub id: String,
    pub title: String,
    pub description: String,
    pub risk: Risk,
    pub entries: Vec<Entry>,
    /// Le groupe peut être supprimé par le logiciel.
    pub removable: bool,
    /// Le groupe est coché dans la sélection recommandée.
    pub recommended: bool,
    /// Avertissement affiché sous le groupe.
    pub note: Option<String>,
}

impl Group {
    pub fn new(id: &str, title: &str, description: &str, risk: Risk) -> Self {
        Group {
            id: id.to_string(),
            title: title.to_string(),
            description: description.to_string(),
            risk,
            entries: Vec::new(),
            removable: true,
            recommended: true,
            note: None,
        }
    }

    pub fn entries(mut self, entries: Vec<Entry>) -> Self {
        self.entries = entries;
        self
    }

    pub fn not_recommended(mut self) -> Self {
        self.recommended = false;
        self
    }

    pub fn info_only(mut self) -> Self {
        self.removable = false;
        self.recommended = false;
        self
    }

    pub fn note(mut self, note: &str) -> Self {
        self.note = Some(note.to_string());
        self
    }

}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileKind {
    Jpeg,
    Png,
    Pdf,
    Word,
    Excel,
    PowerPoint,
    OdfText,
    OdfSheet,
    OdfPresentation,
    OdfOther,
}

impl FileKind {
    pub fn label(&self) -> &'static str {
        match self {
            FileKind::Jpeg => "Image JPEG",
            FileKind::Png => "Image PNG",
            FileKind::Pdf => "Document PDF",
            FileKind::Word => "Document Word",
            FileKind::Excel => "Classeur Excel",
            FileKind::PowerPoint => "Présentation PowerPoint",
            FileKind::OdfText => "Texte LibreOffice (ODT)",
            FileKind::OdfSheet => "Classeur LibreOffice (ODS)",
            FileKind::OdfPresentation => "Présentation LibreOffice (ODP)",
            FileKind::OdfOther => "Document OpenDocument",
        }
    }

    pub fn badge(&self) -> &'static str {
        match self {
            FileKind::Jpeg => "JPG",
            FileKind::Png => "PNG",
            FileKind::Pdf => "PDF",
            FileKind::Word => "DOCX",
            FileKind::Excel => "XLSX",
            FileKind::PowerPoint => "PPTX",
            FileKind::OdfText => "ODT",
            FileKind::OdfSheet => "ODS",
            FileKind::OdfPresentation => "ODP",
            FileKind::OdfOther => "ODF",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Report {
    pub path: PathBuf,
    pub kind: FileKind,
    pub size: u64,
    pub groups: Vec<Group>,
    pub warnings: Vec<String>,
}

impl Report {
    pub fn file_name(&self) -> String {
        self.path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    pub fn total_entries(&self) -> usize {
        self.groups.iter().map(|g| g.entries.len()).sum()
    }

    pub fn max_risk(&self) -> Option<Risk> {
        self.groups.iter().filter(|g| !g.entries.is_empty()).map(|g| g.risk).max()
    }

    pub fn count_risk(&self, risk: Risk) -> usize {
        self.groups.iter().filter(|g| g.risk == risk).count()
    }

    pub fn to_text(&self) -> String {
        let mut s = String::new();
        s.push_str("Rapport de métadonnées Lyra\n");
        s.push_str("===========================\n\n");
        s.push_str(&format!("Fichier : {}\n", self.path.display()));
        s.push_str(&format!("Type    : {}\n", self.kind.label()));
        s.push_str(&format!("Taille  : {}\n", human_size(self.size)));
        s.push_str(&format!(
            "Total   : {} élément(s) dans {} catégorie(s)\n\n",
            self.total_entries(),
            self.groups.len()
        ));
        if self.groups.is_empty() {
            s.push_str("Aucune métadonnée cachée détectée.\n");
        }
        for g in &self.groups {
            s.push_str(&format!(
                "[{}] {} (risque {}){}\n",
                g.id,
                g.title,
                g.risk.label().to_lowercase(),
                if g.removable { "" } else { " | information seulement" }
            ));
            s.push_str(&format!("    {}\n", g.description));
            if let Some(n) = &g.note {
                s.push_str(&format!("    Remarque : {}\n", n));
            }
            for e in &g.entries {
                s.push_str(&format!("    - {} : {}\n", e.key, e.value.replace('\n', " ")));
            }
            s.push('\n');
        }
        for w in &self.warnings {
            s.push_str(&format!("Avertissement : {}\n", w));
        }
        s
    }
}

pub fn human_size(n: u64) -> String {
    let n = n as f64;
    if n < 1024.0 {
        format!("{} o", n)
    } else if n < 1024.0 * 1024.0 {
        format!("{:.1} Ko", n / 1024.0)
    } else {
        format!("{:.1} Mo", n / (1024.0 * 1024.0))
    }
}
