//! Client HTTP épinglé et en-têtes MCP.
//!
//! Toute connexion MCP réseau passe par ici :
//! 1. validation statique de l'URL ([`ih_security::net::validate_url`]) ;
//! 2. résolution DNS, revalidation de **chaque** adresse, puis épinglage des
//!    adresses validées dans le client `reqwest` (`resolve_to_addrs`) : une
//!    réponse DNS ultérieure différente (DNS rebinding) ne peut pas détourner
//!    la connexion ;
//! 3. redirections désactivées et proxys d'environnement ignorés ;
//! 4. en-têtes personnalisés filtrés (jamais `Authorization`, `Cookie`, ni les
//!    en-têtes de contrôle de la connexion).

use ih_core::{IhError, IhResult};
use ih_security::net::{UrlPolicy, UrlTarget};
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

/// Délai de connexion TCP/TLS (le délai global d'une requête est celui du
/// serveur MCP, appliqué au-dessus).
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// En-têtes qu'un utilisateur ne peut pas forger via la configuration : ils
/// porteraient le secret ou casseraient le cadrage HTTP.
const RESERVED_HEADERS: &[&str] = &[
    "authorization",
    "cookie",
    "proxy-authorization",
    "host",
    "content-length",
    "content-type",
    "transfer-encoding",
    "connection",
    "upgrade",
    "te",
    "trailer",
    "accept",
];

/// Valide un nom d'en-tête personnalisé et sa valeur.
pub fn build_custom_headers(
    headers: &std::collections::BTreeMap<String, String>,
) -> IhResult<mcp_reqwest::header::HeaderMap> {
    let mut map = mcp_reqwest::header::HeaderMap::new();
    for (name, value) in headers {
        let name = name.trim();
        if name.is_empty() {
            return Err(IhError::Invalid("nom d'en-tête MCP vide".into()));
        }
        if name.bytes().any(|byte| {
            !byte.is_ascii_alphanumeric() && byte != b'-' && byte != b'_' && byte != b'.'
        }) {
            return Err(IhError::Invalid(format!(
                "nom d'en-tête MCP invalide : {name}"
            )));
        }
        if RESERVED_HEADERS
            .iter()
            .any(|reserved| name.eq_ignore_ascii_case(reserved))
        {
            return Err(IhError::Invalid(format!(
                "en-tête MCP réservé : {name} (utilisez le jeton ou le champ dédié)"
            )));
        }
        if value.contains('\r') || value.contains('\n') {
            return Err(IhError::Invalid(format!(
                "valeur d'en-tête MCP invalide (retour à la ligne) : {name}"
            )));
        }
        let header_name =
            mcp_reqwest::header::HeaderName::from_bytes(name.as_bytes()).map_err(|error| {
                IhError::Invalid(format!("en-tête MCP « {name} » invalide : {error}"))
            })?;
        let header_value =
            mcp_reqwest::header::HeaderValue::from_str(value.trim()).map_err(|error| {
                IhError::Invalid(format!(
                    "valeur de l'en-tête MCP « {name} » invalide : {error}"
                ))
            })?;
        map.insert(header_name, header_value);
    }
    Ok(map)
}

/// URL validée et client HTTP épinglé prêts pour une connexion.
#[derive(Clone)]
pub struct PinnedEndpoint {
    pub target: UrlTarget,
    pub client: mcp_reqwest::Client,
}

impl std::fmt::Debug for PinnedEndpoint {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PinnedEndpoint")
            .field("url", &self.target.url.as_str())
            .finish_non_exhaustive()
    }
}

/// Valide l'URL, résout le DNS, revalide et épingle les adresses.
pub async fn pin_endpoint(raw: &str, policy: UrlPolicy) -> IhResult<PinnedEndpoint> {
    let target = ih_security::net::validate_url(raw, policy)?;
    let lookup = tokio::net::lookup_host((target.host.as_str(), target.port))
        .await
        .map_err(|error| {
            IhError::Denied(format!(
                "résolution DNS impossible pour le serveur MCP {} : {error}",
                target.host
            ))
        })?;
    let mut addresses: Vec<SocketAddr> = lookup.collect();
    if addresses.is_empty() {
        return Err(IhError::Denied(format!(
            "aucune adresse résolue pour le serveur MCP {}",
            target.host
        )));
    }
    addresses.sort_by_key(|addr| (addr.ip().to_string(), addr.port()));
    addresses.dedup_by_key(|addr| (addr.ip(), addr.port()));

    let ips: Vec<IpAddr> = addresses.iter().map(SocketAddr::ip).collect();
    ih_security::net::check_resolved(&target, &ips, policy)?;

    let client = mcp_reqwest::Client::builder()
        .redirect(mcp_reqwest::redirect::Policy::none())
        .no_proxy()
        .connect_timeout(CONNECT_TIMEOUT)
        .user_agent(format!("IH/{}", env!("CARGO_PKG_VERSION")))
        .resolve_to_addrs(target.host.as_str(), &addresses)
        .build()
        .map_err(|error| IhError::Internal(format!("client HTTP MCP impossible : {error}")))?;

    Ok(PinnedEndpoint { target, client })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn en_tetes_x_personnalises_acceptes() {
        let mut headers = BTreeMap::new();
        headers.insert("X-Client".into(), "ih".into());
        headers.insert("X-Trace-Id".into(), "abc-123".into());
        let map = build_custom_headers(&headers).unwrap();
        assert_eq!(map.len(), 2);
        assert_eq!(map.get("x-client").unwrap(), "ih");
    }

    #[test]
    fn en_tetes_reserves_refuses() {
        for reserved in [
            "Authorization",
            "cookie",
            "Host",
            "Content-Length",
            "accept",
            "connection",
        ] {
            let mut headers = BTreeMap::new();
            headers.insert(reserved.into(), "x".into());
            assert!(
                build_custom_headers(&headers).is_err(),
                "{reserved} devrait être refusé"
            );
        }
    }

    #[test]
    fn injection_entete_refusee() {
        let mut headers = BTreeMap::new();
        headers.insert("X-Test".into(), "a\r\nX-Evil: 1".into());
        assert!(build_custom_headers(&headers).is_err());
    }

    #[test]
    fn nom_en_tete_invalide_refuse() {
        let mut headers = BTreeMap::new();
        headers.insert("X Mauvais".into(), "v".into());
        assert!(build_custom_headers(&headers).is_err());
    }

    #[tokio::test]
    async fn epinglage_sur_ip_litterale_locale() {
        let pinned = pin_endpoint("http://127.0.0.1:9/mcp", UrlPolicy::default())
            .await
            .expect("IP locale acceptée");
        assert_eq!(pinned.target.port, 9);
    }

    #[tokio::test]
    async fn epinglage_refuse_metadonnees() {
        let error = pin_endpoint("https://169.254.169.254/latest", UrlPolicy::default())
            .await
            .unwrap_err();
        assert!(error.to_string().contains("link-local"), "{error}");
    }
}
