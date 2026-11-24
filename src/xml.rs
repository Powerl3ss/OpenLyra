//! Petits outils XML : arbre en lecture seule et filtre en flux qui
//! réécrit le document en conservant tout ce qui n'est pas supprimé.

use anyhow::Result;
use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, Writer};

#[derive(Debug, Default, Clone)]
pub struct Node {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Node>,
    pub text: String,
}

impl Node {
    pub fn local(&self) -> &str {
        local_of(&self.name)
    }

    pub fn attr(&self, local: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| local_of(k) == local)
            .map(|(_, v)| v.as_str())
    }

    pub fn all_text(&self) -> String {
        let mut s = self.text.clone();
        for c in &self.children {
            let t = c.all_text();
            if !t.is_empty() {
                if !s.is_empty() && !s.ends_with(' ') {
                    s.push(' ');
                }
                s.push_str(&t);
            }
        }
        s.trim().to_string()
    }

    /// Parcours en profondeur avec la liste des ancêtres.
    pub fn walk<'a>(&'a self, f: &mut dyn FnMut(&'a Node, &[&'a Node])) {
        let mut stack: Vec<&'a Node> = Vec::new();
        walk_inner(self, &mut stack, f);
    }

    pub fn find_all<'a>(&'a self, local: &str) -> Vec<&'a Node> {
        let mut out = Vec::new();
        self.walk(&mut |n, _| {
            if n.local() == local {
                out.push(n);
            }
        });
        out
    }

    /// Premier élément réel (la racine du document).
    pub fn root(&self) -> Option<&Node> {
        self.children.first()
    }
}

fn walk_inner<'a>(n: &'a Node, stack: &mut Vec<&'a Node>, f: &mut dyn FnMut(&'a Node, &[&'a Node])) {
    f(n, stack);
    stack.push(n);
    for c in &n.children {
        walk_inner(c, stack, f);
    }
    stack.pop();
}

pub fn local_of(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

fn node_from(e: &BytesStart) -> Node {
    let name = String::from_utf8_lossy(e.name().as_ref()).into_owned();
    let attrs = e
        .attributes()
        .with_checks(false)
        .flatten()
        .map(|a| {
            let k = String::from_utf8_lossy(a.key.as_ref()).into_owned();
            let v = a
                .unescape_value()
                .map(|c| c.into_owned())
                .unwrap_or_else(|_| String::from_utf8_lossy(&a.value).into_owned());
            (k, v)
        })
        .collect();
    Node { name, attrs, ..Default::default() }
}

pub fn parse(xml: &[u8]) -> Result<Node> {
    let mut r = Reader::from_reader(xml);
    r.trim_text(false);
    r.check_end_names(false);
    let mut buf = Vec::new();
    let mut stack: Vec<Node> = vec![Node { name: "#document".into(), ..Default::default() }];
    loop {
        match r.read_event_into(&mut buf)? {
            Event::Start(e) => stack.push(node_from(&e)),
            Event::Empty(e) => {
                let n = node_from(&e);
                stack.last_mut().unwrap().children.push(n);
            }
            Event::End(_) => {
                if stack.len() > 1 {
                    let n = stack.pop().unwrap();
                    stack.last_mut().unwrap().children.push(n);
                }
            }
            Event::Text(t) => {
                let s = t
                    .unescape()
                    .map(|c| c.into_owned())
                    .unwrap_or_else(|_| String::from_utf8_lossy(&t).into_owned());
                stack.last_mut().unwrap().text.push_str(&s);
            }
            Event::CData(c) => {
                stack.last_mut().unwrap().text.push_str(&String::from_utf8_lossy(&c));
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    while stack.len() > 1 {
        let n = stack.pop().unwrap();
        stack.last_mut().unwrap().children.push(n);
    }
    Ok(stack.pop().unwrap())
}

pub enum Action {
    /// Conserver l'élément.
    Keep,
    /// Supprimer l'élément et tout son contenu.
    Drop,
    /// Supprimer la balise mais conserver son contenu.
    Unwrap,
}

/// Réécrit un document XML en appliquant `f` à chaque élément.
pub fn filter(xml: &[u8], mut f: impl FnMut(&BytesStart) -> Action) -> Result<Vec<u8>> {
    let mut r = Reader::from_reader(xml);
    r.trim_text(false);
    r.check_end_names(false);
    let mut w = Writer::new(Vec::with_capacity(xml.len()));
    let mut buf = Vec::new();
    let mut skip_depth = 0usize;
    let mut open: Vec<bool> = Vec::new();
    loop {
        let ev = r.read_event_into(&mut buf)?;
        match &ev {
            Event::Eof => break,
            Event::Start(e) => {
                if skip_depth > 0 {
                    skip_depth += 1;
                } else {
                    match f(e) {
                        Action::Keep => {
                            open.push(true);
                            w.write_event(ev.clone())?;
                        }
                        Action::Unwrap => open.push(false),
                        Action::Drop => skip_depth = 1,
                    }
                }
            }
            Event::End(_) => {
                if skip_depth > 0 {
                    skip_depth -= 1;
                } else if open.pop().unwrap_or(true) {
                    w.write_event(ev.clone())?;
                }
            }
            Event::Empty(e) => {
                if skip_depth == 0 {
                    if let Action::Keep = f(e) {
                        w.write_event(ev.clone())?;
                    }
                }
            }
            _ => {
                if skip_depth == 0 {
                    w.write_event(ev.clone())?;
                }
            }
        }
        buf.clear();
    }
    Ok(w.into_inner())
}

/// Nom qualifié (préfixe:local) d'une balise.
pub fn qname(e: &BytesStart) -> String {
    String::from_utf8_lossy(e.name().as_ref()).into_owned()
}

pub fn local(e: &BytesStart) -> String {
    String::from_utf8_lossy(e.local_name().as_ref()).into_owned()
}

pub fn attr_value(e: &BytesStart, local_name: &str) -> Option<String> {
    e.attributes().with_checks(false).flatten().find_map(|a| {
        let k = String::from_utf8_lossy(a.key.as_ref()).into_owned();
        if local_of(&k) == local_name {
            Some(
                a.unescape_value()
                    .map(|c| c.into_owned())
                    .unwrap_or_else(|_| String::from_utf8_lossy(&a.value).into_owned()),
            )
        } else {
            None
        }
    })
}

/// Formate une date ISO 8601 (2024-03-05T14:22:00Z) en format français.
pub fn pretty_iso_date(s: &str) -> String {
    let s = s.trim();
    let b = s.as_bytes();
    if b.len() >= 10 && b[4] == b'-' && b[7] == b'-' {
        let (y, m, d) = (&s[0..4], &s[5..7], &s[8..10]);
        let mut out = format!("{}/{}/{}", d, m, y);
        if b.len() >= 16 && (b[10] == b'T' || b[10] == b' ') {
            out.push_str(&format!(" à {}", &s[11..16]));
            if s.ends_with('Z') {
                out.push_str(" (UTC)");
            }
        }
        return out;
    }
    s.to_string()
}
