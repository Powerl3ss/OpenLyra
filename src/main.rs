// En version finale sous Windows, pas de console noire derrière la fenêtre.
#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]

mod exifmeta;
mod formats;
mod gui;
mod model;
mod updater;
mod xml;
mod xmp;

use std::collections::HashSet;
use std::path::PathBuf;

fn main() {
    updater::remember_exe_path();
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(|a| a == "update").unwrap_or(false) {
        std::process::exit(cli_update());
    }
    if args.get(1).map(|a| a == "--version").unwrap_or(false) {
        println!("Lyra {}", updater::VERSION);
        return;
    }
    if args.len() >= 3 && (args[1] == "analyze" || args[1] == "clean") {
        std::process::exit(cli(&args));
    }
    // Un fichier passé en argument (« Ouvrir avec… ») est analysé au démarrage.
    let initial = args.get(1).map(PathBuf::from).filter(|p| p.is_file());
    if let Err(e) = gui::run(initial) {
        eprintln!("Erreur de l'interface : {e}");
    }
}

/// lyra update : met à jour le programme depuis GitHub.
fn cli_update() -> i32 {
    println!("Version actuelle : {}", updater::VERSION);
    match updater::check() {
        Ok(None) => {
            println!("Lyra est à jour.");
            0
        }
        Ok(Some(rel)) => {
            println!("Nouvelle version {} : téléchargement…", rel.version);
            match updater::install(&rel, &|_, _| {}) {
                Ok(()) => {
                    println!("Version {} installée.", rel.version);
                    0
                }
                Err(e) => {
                    eprintln!("Erreur : {e:#}");
                    1
                }
            }
        }
        Err(e) => {
            eprintln!("Erreur : {e:#}");
            1
        }
    }
}

/// Mode ligne de commande :
///   lyra analyze <fichier>
///   lyra clean <fichier> [--all] [-o sortie]
fn cli(args: &[String]) -> i32 {
    let path = PathBuf::from(&args[2]);
    let report = match formats::analyze_file(&path) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Erreur : {e:#}");
            return 1;
        }
    };
    if args[1] == "analyze" {
        print!("{}", report.to_text());
        return 0;
    }
    let all = args.iter().any(|a| a == "--all");
    // --groups id1,id2 : ne nettoyer que ces catégories
    let only: Option<HashSet<String>> = args
        .iter()
        .position(|a| a == "--groups")
        .and_then(|i| args.get(i + 1))
        .map(|v| v.split(',').map(|s| s.trim().to_string()).collect());
    let sel: HashSet<String> = report
        .groups
        .iter()
        .filter(|g| match &only {
            Some(o) => g.removable && o.contains(&g.id),
            None => g.removable && (all || g.recommended),
        })
        .map(|g| g.id.clone())
        .collect();
    let out = args
        .iter()
        .position(|a| a == "-o")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from)
        .unwrap_or_else(|| formats::default_output(&path));
    let bytes = std::fs::read(&path).unwrap();
    match formats::clean_bytes(&bytes, report.kind, &sel) {
        Ok((data, notes)) => {
            if let Err(e) = std::fs::write(&out, data) {
                eprintln!("Écriture impossible : {e}");
                return 1;
            }
            println!("Supprimé : {:?}", sel);
            for n in notes {
                println!("Note : {n}");
            }
            println!("Fichier nettoyé : {}", out.display());
            0
        }
        Err(e) => {
            eprintln!("Erreur : {e:#}");
            1
        }
    }
}
