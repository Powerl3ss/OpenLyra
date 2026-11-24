//! Mise à jour automatique depuis les releases GitHub du dépôt OpenLyra.
//!
//! 1. On interroge l'API GitHub pour connaître la dernière version publiée.
//! 2. Si elle est plus récente, on télécharge l'exécutable correspondant à
//!    ce système, on vérifie son empreinte SHA-256, puis on remplace le
//!    programme en cours d'exécution.
//! 3. L'utilisateur redémarre Lyra pour lancer la nouvelle version.

use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

pub const REPO: &str = "Powerl3ss/OpenLyra";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Nom du fichier publié dans chaque release, selon le système.
pub fn asset_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "Lyra-windows.exe"
    } else if cfg!(target_os = "macos") {
        "Lyra-macos"
    } else {
        "Lyra-linux"
    }
}

#[derive(Clone, Debug)]
pub struct Release {
    pub version: String,
    pub notes: String,
    pub page: String,
    pub asset_url: String,
    pub sha_url: Option<String>,
    pub size: u64,
}

#[derive(Deserialize)]
struct ApiRelease {
    tag_name: String,
    #[serde(default)]
    body: Option<String>,
    html_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    assets: Vec<ApiAsset>,
}

#[derive(Deserialize)]
struct ApiAsset {
    name: String,
    browser_download_url: String,
    size: u64,
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(60))
        .user_agent(&format!("Lyra/{VERSION}"))
        .build()
}

fn api_base() -> String {
    // Variable utile uniquement pour les tests.
    std::env::var("LYRA_UPDATE_API").unwrap_or_else(|_| "https://api.github.com".into())
}

/// Convertit « v1.2.3 » en (1, 2, 3).
pub fn parse_version(s: &str) -> Option<(u64, u64, u64)> {
    let s = s.trim().trim_start_matches(['v', 'V']);
    let core = s.split(['-', '+']).next()?;
    let mut it = core.split('.').map(|p| p.parse::<u64>());
    let major = it.next()?.ok()?;
    let minor = it.next().unwrap_or(Ok(0)).ok()?;
    let patch = it.next().unwrap_or(Ok(0)).ok()?;
    Some((major, minor, patch))
}

pub fn is_newer(remote: &str, local: &str) -> bool {
    match (parse_version(remote), parse_version(local)) {
        (Some(r), Some(l)) => r > l,
        _ => false,
    }
}

static EXE_PATH: OnceLock<Option<PathBuf>> = OnceLock::new();

/// Mémorise le chemin du programme. À appeler au démarrage, avant tout
/// remplacement : ensuite, le système peut ne plus retrouver le fichier d'origine.
pub fn remember_exe_path() {
    EXE_PATH.get_or_init(|| std::env::current_exe().ok());
}

fn exe_path() -> Result<PathBuf> {
    if let Some(Some(p)) = EXE_PATH.get() {
        return Ok(p.clone());
    }
    let p = std::env::current_exe()?;
    let s = p.to_string_lossy();
    Ok(match s.strip_suffix(" (deleted)") {
        Some(clean) => PathBuf::from(clean),
        None => p,
    })
}

/// Renvoie la release la plus récente si elle est plus récente que la version actuelle.
pub fn check() -> Result<Option<Release>> {
    let url = format!("{}/repos/{}/releases/latest", api_base(), REPO);
    let resp = agent()
        .get(&url)
        .set("Accept", "application/vnd.github+json")
        .call();
    let resp = match resp {
        Ok(r) => r,
        Err(ureq::Error::Status(404, _)) => return Ok(None), // aucune release publiée
        Err(ureq::Error::Status(403, _)) => bail!("GitHub limite temporairement les vérifications, réessaie dans une heure."),
        Err(e) => return Err(anyhow!("Impossible de joindre GitHub : {e}")),
    };
    let rel: ApiRelease = resp.into_json().context("Réponse de GitHub illisible")?;
    if rel.draft || rel.prerelease || !is_newer(&rel.tag_name, VERSION) {
        return Ok(None);
    }
    let name = asset_name();
    let asset = rel
        .assets
        .iter()
        .find(|a| a.name.eq_ignore_ascii_case(name))
        .ok_or_else(|| anyhow!("La version {} ne contient pas de fichier pour ce système ({name}).", rel.tag_name))?;
    let sha = rel
        .assets
        .iter()
        .find(|a| a.name.eq_ignore_ascii_case(&format!("{name}.sha256")))
        .map(|a| a.browser_download_url.clone());
    Ok(Some(Release {
        version: rel.tag_name.trim_start_matches(['v', 'V']).to_string(),
        notes: rel.body.unwrap_or_default(),
        page: rel.html_url,
        asset_url: asset.browser_download_url.clone(),
        sha_url: sha,
        size: asset.size,
    }))
}

/// Télécharge, vérifie et installe la nouvelle version.
/// `progress(reçu, total)` est appelé pendant le téléchargement.
pub fn install(rel: &Release, progress: &dyn Fn(u64, u64)) -> Result<()> {
    let agent = agent();
    let resp = agent
        .get(&rel.asset_url)
        .call()
        .map_err(|e| anyhow!("Téléchargement impossible : {e}"))?;
    let total = resp
        .header("Content-Length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(rel.size);

    let tmp = std::env::temp_dir().join(format!("lyra-update-{}{}", rel.version, std::env::consts::EXE_SUFFIX));
    let mut file = std::fs::File::create(&tmp).context("Impossible de créer le fichier temporaire")?;
    let mut reader = resp.into_reader();
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut done = 0u64;
    loop {
        let n = reader.read(&mut buf).context("Téléchargement interrompu")?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
        hasher.update(&buf[..n]);
        done += n as u64;
        progress(done, total);
    }
    file.flush()?;
    drop(file);

    if total > 0 && done != total {
        let _ = std::fs::remove_file(&tmp);
        bail!("Téléchargement incomplet ({done} octets sur {total}).");
    }

    if let Some(sha_url) = &rel.sha_url {
        let expected = agent
            .get(sha_url)
            .call()
            .map_err(|e| anyhow!("Empreinte de vérification introuvable : {e}"))?
            .into_string()?;
        let expected = expected.split_whitespace().next().unwrap_or("").to_ascii_lowercase();
        let actual: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
        if expected != actual {
            let _ = std::fs::remove_file(&tmp);
            bail!("Le fichier téléchargé est corrompu (empreinte SHA-256 différente). Mise à jour annulée.");
        }
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
    }

    self_replace::self_replace(&tmp).context("Impossible de remplacer le programme (droits insuffisants ?)")?;
    let _ = std::fs::remove_file(&tmp);
    Ok(())
}

/// Relance Lyra (nouvelle version) et ferme l'instance actuelle.
pub fn restart() -> Result<()> {
    let exe = exe_path()?;
    std::process::Command::new(exe).spawn()?;
    std::process::exit(0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert_eq!(parse_version("v1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_version("2.0"), Some((2, 0, 0)));
        assert!(is_newer("v1.0.1", "1.0.0"));
        assert!(is_newer("v1.10.0", "1.9.9"));
        assert!(!is_newer("v1.0.0", "1.0.0"));
        assert!(!is_newer("v0.9.0", "1.0.0"));
        assert!(!is_newer("nimportequoi", "1.0.0"));
    }
}
