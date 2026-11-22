//! Détection de la plateforme et calibration matérielle.
//!
//! IH cible Zorin OS, Ubuntu/Debian et Arch Linux sur un ASUS TUF A15
//! (16 Go de RAM, GTX 1660 Ti). Ces informations servent à proposer des
//! limites de ressources réalistes et à adapter les instructions
//! d'installation — jamais à autoriser une action.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DistroFamily {
    Arch,
    Debian,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlatformInfo {
    pub os_id: String,
    pub os_name: String,
    pub os_version: String,
    pub family: DistroFamily,
    pub arch: String,
    pub kernel: String,
    pub cpu_cores: usize,
    pub ram_mb: u64,
    pub gpu: Option<String>,
    pub recommended_max_agents: u8,
}

impl PlatformInfo {
    pub fn detect() -> Self {
        let os =
            parse_os_release(&read_to_string(Path::new("/etc/os-release")).unwrap_or_default());
        let family = family_from(&os);
        let ram_mb = total_ram_mb();
        Self {
            os_id: os.get("ID").cloned().unwrap_or_else(|| "linux".into()),
            os_name: os
                .get("PRETTY_NAME")
                .cloned()
                .or_else(|| os.get("NAME").cloned())
                .unwrap_or_else(|| "Linux".into()),
            os_version: os.get("VERSION_ID").cloned().unwrap_or_default(),
            family,
            arch: std::env::consts::ARCH.to_string(),
            kernel: kernel_version(),
            cpu_cores: std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1),
            ram_mb,
            gpu: detect_gpu(),
            recommended_max_agents: recommend_agents(ram_mb),
        }
    }

    /// Commande d'installation de la toolchain adaptée à la distribution.
    pub fn toolchain_hint(&self) -> &'static str {
        match self.family {
            DistroFamily::Arch => "sudo pacman -S --needed base-devel rust",
            DistroFamily::Debian => {
                "sudo apt install build-essential libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev"
            }
            DistroFamily::Other => "installer Rust stable et les dépendances WebKitGTK",
        }
    }
}

fn read_to_string(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

/// Parse un fichier `os-release` au format `CLE="valeur"`.
pub fn parse_os_release(content: &str) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            let value = value
                .trim()
                .trim_matches('"')
                .trim_matches('\'')
                .to_string();
            map.insert(key.trim().to_string(), value);
        }
    }
    map
}

fn family_from(os: &BTreeMap<String, String>) -> DistroFamily {
    let id = os
        .get("ID")
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();
    let like = os
        .get("ID_LIKE")
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();
    let joined = format!("{id} {like}");
    if joined.contains("arch") {
        DistroFamily::Arch
    } else if joined.contains("debian")
        || joined.contains("ubuntu")
        || joined.contains("zorin")
        || joined.contains("linuxmint")
        || joined.contains("pop")
    {
        DistroFamily::Debian
    } else {
        DistroFamily::Other
    }
}

fn kernel_version() -> String {
    std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

fn total_ram_mb() -> u64 {
    let Ok(meminfo) = std::fs::read_to_string("/proc/meminfo") else {
        return 0;
    };
    meminfo
        .lines()
        .find_map(|line| {
            let rest = line.strip_prefix("MemTotal:")?;
            let kb: u64 = rest.split_whitespace().next()?.parse().ok()?;
            Some(kb / 1024)
        })
        .unwrap_or(0)
}

/// Détecte un GPU sans dépendre de `nvidia-smi` : lecture de sysfs puis PATH.
fn detect_gpu() -> Option<String> {
    if let Some(model) = nvidia_from_proc() {
        return Some(model);
    }
    if let Ok(entries) = std::fs::read_dir("/sys/class/drm") {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.starts_with("card") || name.contains('-') {
                continue;
            }
            let uevent = entry.path().join("device/uevent");
            if let Some(content) = read_to_string(&uevent) {
                for line in content.lines() {
                    if let Some(driver) = line.strip_prefix("DRIVER=") {
                        let driver = driver.trim();
                        if !driver.is_empty() {
                            return Some(format!("{driver} ({name})"));
                        }
                    }
                }
            }
        }
    }
    None
}

fn nvidia_from_proc() -> Option<String> {
    let content = read_to_string(Path::new("/proc/driver/nvidia/version"))?;
    let first = content
        .lines()
        .next()
        .unwrap_or("NVIDIA")
        .trim()
        .to_string();
    Some(format!("NVIDIA — {first}"))
}

fn recommend_agents(ram_mb: u64) -> u8 {
    match ram_mb {
        0..=7_999 => 1,
        8_000..=15_999 => 2,
        16_000..=31_999 => 2,
        _ => 3,
    }
}

/// Cherche un exécutable dans `PATH` sans dépendance externe.
pub fn find_in_path(program: &str) -> Option<PathBuf> {
    if program.contains('/') {
        let p = PathBuf::from(program);
        return p.is_file().then_some(p);
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(program))
        .find(|candidate| candidate.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_os_release_zorin() {
        let os = parse_os_release(
            "NAME=\"Zorin OS\"\nID=zorin\nID_LIKE=\"ubuntu debian\"\nVERSION_ID=\"18\"\nPRETTY_NAME=\"Zorin OS 18.1\"\n",
        );
        assert_eq!(os.get("ID").unwrap(), "zorin");
        assert_eq!(os.get("PRETTY_NAME").unwrap(), "Zorin OS 18.1");
        assert_eq!(family_from(&os), DistroFamily::Debian);
    }

    #[test]
    fn parse_os_release_arch() {
        let os = parse_os_release("ID=arch\nNAME=\"Arch Linux\"\n");
        assert_eq!(family_from(&os), DistroFamily::Arch);
    }

    #[test]
    fn parse_os_release_inconnu() {
        let os = parse_os_release("ID=gentoo\n");
        assert_eq!(family_from(&os), DistroFamily::Other);
    }

    #[test]
    fn detection_reelle_ne_panique_pas() {
        let info = PlatformInfo::detect();
        assert!(!info.os_id.is_empty());
        assert!(info.cpu_cores >= 1);
        assert!((1..=3).contains(&info.recommended_max_agents));
    }

    #[test]
    fn recommandation_selon_ram() {
        assert_eq!(recommend_agents(4_000), 1);
        assert_eq!(recommend_agents(16_000), 2);
        assert_eq!(recommend_agents(32_000), 3);
    }

    #[test]
    fn recherche_dans_path() {
        assert!(find_in_path("sh").is_some());
        assert!(find_in_path("executable-inexistant-ih").is_none());
    }
}
