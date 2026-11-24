//! Politique réseau des serveurs MCP distants (anti-SSRF).
//!
//! Règles appliquées **avant** toute connexion et **revalidées après
//! résolution DNS** (l'appelant résout, puis appelle [`check_resolved`]) :
//!
//! - schémas acceptés : `https` partout, `http` **uniquement** vers le
//!   loopback ou les réseaux privés (serveurs MCP de développement) ;
//! - URL invalide, identifiants dans l'URL (`user:pass@`) ou fragment :
//!   refusés ;
//! - adresses bloquées dans tous les cas : link-local `169.254.0.0/16`
//!   (dont l'endpoint de métadonnées cloud `169.254.169.254`), `0.0.0.0/8`,
//!   multicast, plages réservées/benchmark/doc, `::`, `fe80::/10`, `ff00::/8`,
//!   et leurs équivalents IPv4-mappés (`::ffff:a.b.c.d`) ;
//! - l'appelant épingle ensuite les IP validées dans le client HTTP, ce qui
//!   neutralise le DNS rebinding entre résolution et connexion.
//!
//! Ce module est volontairement synchrone : la résolution DNS reste dans la
//! couche qui possède le runtime (crate `ih-mcp`), qui doit re-valider les
//! adresses obtenues ici.

use ih_core::{IhError, IhResult};
use serde::{Deserialize, Serialize};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use url::Url;

/// Classe d'une destination résolue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostClass {
    /// `127.0.0.0/8`, `::1`.
    Loopback,
    /// Réseaux privés (`10/8`, `172.16/12`, `192.168/16`, ULA `fc00::/7`).
    Private,
    /// Toute autre adresse publique.
    Public,
}

impl HostClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Loopback => "loopback",
            Self::Private => "private",
            Self::Public => "public",
        }
    }
}

/// Politique configurable. Par défaut : loopback et réseaux privés autorisés
/// (les serveurs MCP de développement tournent souvent en local), `http`
/// refusé vers Internet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UrlPolicy {
    /// Autoriser `loopback` (127.0.0.1/::1) — défaut : vrai.
    pub allow_loopback: bool,
    /// Autoriser les réseaux privés RFC1918/ULA — défaut : vrai.
    pub allow_private_networks: bool,
    /// Autoriser `http://` (sans TLS) vers loopback/privé seulement — défaut : vrai.
    pub allow_http_private: bool,
}

impl Default for UrlPolicy {
    fn default() -> Self {
        Self {
            allow_loopback: true,
            allow_private_networks: true,
            allow_http_private: true,
        }
    }
}

/// URL validée statiquement (avant résolution DNS).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UrlTarget {
    pub url: Url,
    pub scheme: String,
    pub host: String,
    pub port: u16,
}

/// Vérifie une URL de serveur MCP : schéma, identifiants, fragment, hôte.
///
/// Ne résout pas le DNS : l'appelant doit ensuite appeler [`check_resolved`]
/// avec les adresses réellement obtenues, puis connecter en épinglant ces
/// adresses.
pub fn validate_url(raw: &str, policy: UrlPolicy) -> IhResult<UrlTarget> {
    if raw.trim().is_empty() {
        return Err(IhError::Invalid("URL de serveur MCP vide".into()));
    }
    let url = Url::parse(raw.trim())
        .map_err(|error| IhError::Invalid(format!("URL de serveur MCP invalide : {error}")))?;

    let scheme = url.scheme().to_string();
    if scheme != "https" && scheme != "http" {
        return Err(IhError::Invalid(format!(
            "schéma refusé pour un serveur MCP : {scheme} (attendu https, ou http local)"
        )));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(IhError::Invalid(
            "une URL de serveur MCP ne doit pas contenir d'identifiants".into(),
        ));
    }
    if url.fragment().is_some() {
        return Err(IhError::Invalid(
            "une URL de serveur MCP ne doit pas contenir de fragment".into(),
        ));
    }
    let host = url
        .host_str()
        .filter(|host| !host.is_empty())
        .ok_or_else(|| IhError::Invalid("URL de serveur MCP sans hôte".into()))?
        .to_string();
    if host.contains(' ') {
        return Err(IhError::Invalid(format!(
            "hôte de serveur MCP invalide : {host}"
        )));
    }
    let port = url
        .port_or_known_default()
        .ok_or_else(|| IhError::Invalid("port de serveur MCP indéterminable".into()))?;

    let target = UrlTarget {
        url,
        scheme,
        host,
        port,
    };

    // Une IP littérale peut être vérifiée tout de suite : inutile de laisser
    // passer une destination interdite jusqu'à la résolution.
    if let Ok(ip) = target.host.parse::<IpAddr>() {
        check_resolved(&target, &[ip], policy)?;
    }

    Ok(target)
}

/// Revalide la destination après résolution DNS.
///
/// Chaque adresse doit être autorisée : une seule adresse interdite suffit à
/// refuser la connexion (un attaquant ne doit pas gagner en ajoutant une IP
/// publique à côté d'une IP de métadonnées).
pub fn check_resolved(
    target: &UrlTarget,
    addrs: &[IpAddr],
    policy: UrlPolicy,
) -> IhResult<Vec<IpAddr>> {
    if addrs.is_empty() {
        return Err(IhError::Denied(format!(
            "résolution DNS vide pour {}",
            target.host
        )));
    }
    let mut classes = Vec::with_capacity(addrs.len());
    for ip in addrs {
        let class = classify_ip(*ip).map_err(|reason| {
            IhError::Denied(format!(
                "destination refusée pour le serveur MCP {} ({ip}) : {reason}",
                target.host
            ))
        })?;
        classes.push(class);
    }

    // Toutes les adresses doivent appartenir à une même classe de risque
    // acceptable pour le schéma demandé.
    for (ip, class) in addrs.iter().zip(classes.iter()) {
        match class {
            HostClass::Loopback if !policy.allow_loopback => {
                return Err(IhError::Denied(format!(
                    "loopback interdit par la politique : {ip}"
                )));
            }
            HostClass::Private if !policy.allow_private_networks => {
                return Err(IhError::Denied(format!(
                    "réseau privé interdit par la politique : {ip}"
                )));
            }
            _ => {}
        }
        if target.scheme == "http" {
            if !policy.allow_http_private || *class == HostClass::Public {
                return Err(IhError::Denied(format!(
                    "http non chiffré refusé vers {ip} (seuls loopback et réseaux privés l'autorisent)"
                )));
            }
            if *class == HostClass::Loopback && !policy.allow_loopback {
                return Err(IhError::Denied(format!("loopback interdit : {ip}")));
            }
        }
    }

    Ok(addrs.to_vec())
}

/// Classe une adresse, ou explique pourquoi elle est interdite.
pub fn classify_ip(ip: IpAddr) -> Result<HostClass, &'static str> {
    match ip {
        IpAddr::V4(v4) => classify_ipv4(v4),
        IpAddr::V6(v6) => classify_ipv6(v6),
    }
}

fn classify_ipv4(ip: Ipv4Addr) -> Result<HostClass, &'static str> {
    let octets = ip.octets();
    if ip.is_unspecified() {
        return Err("adresse non spécifiée (0.0.0.0)");
    }
    if ip.is_loopback() {
        return Ok(HostClass::Loopback);
    }
    if ip.is_link_local() {
        // 169.254.0.0/16, dont l'endpoint de métadonnées cloud.
        return Err("adresse link-local (métadonnées cloud)");
    }
    if ip.is_multicast() {
        return Err("adresse multicast");
    }
    if ip.is_broadcast() {
        return Err("adresse de diffusion");
    }
    // Plages réservées, benchmark, documentation, CGNAT : jamais un serveur MCP.
    match octets[0] {
        0 => return Err("réseau 0.0.0.0/8 réservé"),
        100 if (64..=127).contains(&octets[1]) => return Err("plage CGNAT 100.64.0.0/10"),
        127 => return Ok(HostClass::Loopback),
        192 if octets[1] == 0 && octets[2] == 0 => return Err("plage 192.0.0.0/24 réservée"),
        192 if octets[1] == 0 && octets[2] == 2 => return Err("documentation 192.0.2.0/24"),
        198 if octets[1] == 18 || octets[1] == 19 => return Err("benchmark 198.18.0.0/15"),
        198 if octets[1] == 51 && octets[2] == 100 => return Err("documentation 198.51.100.0/24"),
        203 if octets[1] == 0 && octets[2] == 113 => return Err("documentation 203.0.113.0/24"),
        240..=255 => return Err("plage réservée 240.0.0.0/4"),
        _ => {}
    }
    if ip.is_private() {
        return Ok(HostClass::Private);
    }
    Ok(HostClass::Public)
}

fn classify_ipv6(ip: Ipv6Addr) -> Result<HostClass, &'static str> {
    if ip.is_unspecified() {
        return Err("adresse non spécifiée (::)");
    }
    if ip.is_loopback() {
        return Ok(HostClass::Loopback);
    }
    let segments = ip.segments();
    if segments[0] & 0xffc0 == 0xfe80 {
        return Err("adresse link-local fe80::/10");
    }
    if segments[0] & 0xff00 == 0xff00 {
        return Err("adresse multicast ff00::/8");
    }
    if segments[0] == 0x2001 && segments[1] == 0x0db8 {
        return Err("documentation 2001:db8::/32");
    }
    // IPv4 mappée : évaluer comme IPv4 (anti-contournement).
    if let Some(v4) = ip.to_ipv4_mapped() {
        return classify_ipv4(v4);
    }
    if segments[0] & 0xfe00 == 0xfc00 {
        return Ok(HostClass::Private);
    }
    Ok(HostClass::Public)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(raw: &str) -> UrlTarget {
        validate_url(raw, UrlPolicy::default()).unwrap()
    }

    #[test]
    fn https_public_accepte() {
        let t = target("https://mcp.exemple.com/api");
        assert_eq!(t.scheme, "https");
        assert_eq!(t.host, "mcp.exemple.com");
        assert_eq!(t.port, 443);
        check_resolved(
            &t,
            &["93.184.216.34".parse().unwrap()],
            UrlPolicy::default(),
        )
        .unwrap();
    }

    #[test]
    fn http_public_refuse() {
        let t = target("http://mcp.exemple.com/api");
        let error = check_resolved(
            &t,
            &["93.184.216.34".parse().unwrap()],
            UrlPolicy::default(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("non chiffré"), "{error}");
    }

    #[test]
    fn http_loopback_accepte() {
        let t = target("http://127.0.0.1:8642/mcp");
        assert_eq!(t.port, 8642);
        check_resolved(&t, &["127.0.0.1".parse().unwrap()], UrlPolicy::default()).unwrap();
    }

    #[test]
    fn http_prive_accepte() {
        let t = target("http://192.168.1.10:8080/mcp");
        check_resolved(&t, &["192.168.1.10".parse().unwrap()], UrlPolicy::default()).unwrap();
    }

    #[test]
    fn metadonnees_cloud_bloquees() {
        // IP littérale : refusée dès la validation de l'URL.
        assert!(validate_url("https://169.254.169.254/latest", UrlPolicy::default()).is_err());
        // Nom résolu vers l'endpoint de métadonnées : refusé après résolution.
        let t = target("https://metadata.exemple.com/latest");
        let error = check_resolved(
            &t,
            &["169.254.169.254".parse().unwrap()],
            UrlPolicy::default(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("link-local"), "{error}");
    }

    #[test]
    fn adresses_reservees_bloquees() {
        for ip in [
            "0.0.0.0",
            "224.0.0.1",
            "255.255.255.255",
            "100.64.0.1",
            "198.18.0.1",
            "::",
            "fe80::1",
            "ff02::1",
            "2001:db8::1",
        ] {
            let parsed: IpAddr = ip.parse().unwrap();
            assert!(classify_ip(parsed).is_err(), "{ip} devrait être refusé");
        }
    }

    #[test]
    fn ipv4_mappee_dans_ipv6_evaluee_comme_ipv4() {
        let mapped: IpAddr = "::ffff:169.254.169.254".parse().unwrap();
        assert!(classify_ip(mapped).is_err());
        let mapped_loop: IpAddr = "::ffff:127.0.0.1".parse().unwrap();
        assert_eq!(classify_ip(mapped_loop).unwrap(), HostClass::Loopback);
    }

    #[test]
    fn ipv6_ula_privee_et_loopback() {
        assert_eq!(
            classify_ip("fc00::1".parse().unwrap()).unwrap(),
            HostClass::Private
        );
        assert_eq!(
            classify_ip("::1".parse().unwrap()).unwrap(),
            HostClass::Loopback
        );
    }

    #[test]
    fn url_malformee_refusee() {
        for raw in [
            "",
            "pas une url",
            "gopher://exemple.com/",
            "file:///etc/passwd",
            "https://user:pass@exemple.com/mcp",
            "https://exemple.com/mcp#fragment",
        ] {
            assert!(
                validate_url(raw, UrlPolicy::default()).is_err(),
                "{raw:?} devrait être refusé"
            );
        }
    }

    #[test]
    fn ip_litterale_interdite_refusee_des_la_validation() {
        assert!(validate_url("https://169.254.169.254/mcp", UrlPolicy::default()).is_err());
        assert!(validate_url("http://0.0.0.0:8080/mcp", UrlPolicy::default()).is_err());
    }

    #[test]
    fn une_ip_interdite_parmi_des_ips_publiques_refuse_l_ensemble() {
        // DNS rebinding : une seule adresse interdite suffit.
        let t = target("https://mcp.exemple.com/mcp");
        let mixed = vec![
            "93.184.216.34".parse().unwrap(),
            "169.254.169.254".parse().unwrap(),
        ];
        assert!(check_resolved(&t, &mixed, UrlPolicy::default()).is_err());
    }

    #[test]
    fn politique_stricte_interdit_loopback_et_prive() {
        let strict = UrlPolicy {
            allow_loopback: false,
            allow_private_networks: false,
            allow_http_private: false,
        };
        let t = target("http://127.0.0.1:9000/mcp");
        assert!(check_resolved(&t, &["127.0.0.1".parse().unwrap()], strict).is_err());
        let t = target("https://10.0.0.5/mcp");
        assert!(check_resolved(&t, &["10.0.0.5".parse().unwrap()], strict).is_err());
    }
}
