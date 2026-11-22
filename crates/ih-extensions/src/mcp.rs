//! Client MCP minimal : JSON-RPC 2.0 sur stdio.
//!
//! Le transport suit le cadrage MCP actuel pour stdio : un objet JSON par
//! ligne, terminé par `\n` (NDJSON), **sans** en-tête `Content-Length`. Les
//! notifications du serveur (sans `id`) sont ignorées pendant l'attente d'une
//! réponse, et chaque requête a son propre délai maximal.
//!
//! Le client ne fait aucun accès réseau : il lance un processus enfant et
//! communique par ses tubes. Un serveur muet, trop bavard, qui répond une
//! erreur JSON-RPC ou qui meurt produit toujours une [`IhError::Extension`]
//! explicite, jamais une panique.

use ih_core::{IhError, IhResult};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

/// Taille maximale d'un message lu sur la sortie du serveur (8 Mio).
pub const MAX_MESSAGE_BYTES: usize = 8 * 1024 * 1024;

/// Version du protocole MCP annoncée par le client.
pub const MCP_PROTOCOL_VERSION: &str = "2024-11-05";

/// Délai par défaut d'une requête MCP.
pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Configuration d'un serveur MCP lancé en stdio.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpServerConfig {
    /// Identifiant local du serveur (logs et messages d'erreur).
    pub id: String,
    /// Exécutable à lancer (chemin absolu ou nom résolu par `PATH`).
    pub command: String,
    pub args: Vec<String>,
    /// Variables d'environnement ajoutées à celles du processus courant.
    pub env: BTreeMap<String, String>,
}

/// Outil exposé par un serveur MCP.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McpTool {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(rename = "inputSchema", default = "default_input_schema")]
    pub input_schema: Value,
}

fn default_input_schema() -> Value {
    json!({"type": "object"})
}

/// Client MCP connecté à un processus enfant.
///
/// Le champ interne est privé : la seule façon d'interagir passe par les
/// méthodes publiques, ce qui garantit le respect du délai et du cadrage.
pub struct McpClient {
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    server_id: String,
    next_id: u64,
    timeout: Duration,
    dead: bool,
}

impl std::fmt::Debug for McpClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("McpClient")
            .field("server_id", &self.server_id)
            .field("next_id", &self.next_id)
            .field("timeout", &self.timeout)
            .field("dead", &self.dead)
            .finish_non_exhaustive()
    }
}

/// Résultat interne d'un échange : erreur JSON-RPC (serveur vivant) ou erreur
/// de transport (le client devient inutilisable).
enum ExchangeError {
    Rpc(IhError),
    Fatal(IhError),
}

impl McpClient {
    /// Lance un serveur MCP avec le délai par défaut (30 s).
    pub async fn spawn(config: &McpServerConfig) -> IhResult<Self> {
        Self::spawn_with_timeout(config, DEFAULT_REQUEST_TIMEOUT).await
    }

    /// Lance un serveur MCP avec un délai de requête personnalisé.
    pub async fn spawn_with_timeout(config: &McpServerConfig, timeout: Duration) -> IhResult<Self> {
        if config.command.trim().is_empty() {
            return Err(extension(format!(
                "commande vide pour le serveur MCP « {} »",
                config.id
            )));
        }
        if timeout.is_zero() {
            return Err(extension(
                "le délai des requêtes MCP doit être supérieur à zéro",
            ));
        }

        let mut command = Command::new(&config.command);
        command
            .args(&config.args)
            .envs(&config.env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);

        let mut child = command.spawn().map_err(|error| {
            extension(format!(
                "impossible de lancer le serveur MCP « {} » ({}) : {error}",
                config.id, config.command
            ))
        })?;

        let stdin = child.stdin.take().ok_or_else(|| {
            extension(format!(
                "le serveur MCP « {} » n'expose pas d'entrée standard",
                config.id
            ))
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            extension(format!(
                "le serveur MCP « {} » n'expose pas de sortie standard",
                config.id
            ))
        })?;

        tracing::debug!(
            server = %config.id,
            command = %config.command,
            "serveur MCP lancé"
        );

        Ok(Self {
            child: Some(child),
            stdin: Some(stdin),
            stdout: BufReader::new(stdout),
            server_id: config.id.clone(),
            next_id: 0,
            timeout,
            dead: false,
        })
    }

    /// Effectue la poignée de main MCP et renvoie `serverInfo`/`capabilities`.
    ///
    /// Envoie la requête `initialize` puis la notification
    /// `notifications/initialized`, comme l'exige le protocole.
    pub async fn initialize(&mut self, client_name: &str, client_version: &str) -> IhResult<Value> {
        if client_name.trim().is_empty() {
            return Err(extension("le nom du client MCP est vide"));
        }
        if client_version.trim().is_empty() {
            return Err(extension("la version du client MCP est vide"));
        }

        let params = json!({
            "protocolVersion": MCP_PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": {
                "name": client_name,
                "version": client_version,
            },
        });
        let result = self.request("initialize", Some(params)).await?;
        self.send_notification("notifications/initialized", json!({}))
            .await?;
        Ok(result)
    }

    /// Récupère la liste des outils exposés par le serveur.
    pub async fn list_tools(&mut self) -> IhResult<Vec<McpTool>> {
        let result = self.request("tools/list", Some(json!({}))).await?;
        let items = result
            .get("tools")
            .ok_or_else(|| {
                extension(format!(
                    "réponse tools/list du serveur MCP « {} » sans tableau « tools »",
                    self.server_id
                ))
            })?
            .as_array()
            .ok_or_else(|| {
                extension(format!(
                    "réponse tools/list du serveur MCP « {} » : « tools » n'est pas un tableau",
                    self.server_id
                ))
            })?;

        let mut tools = Vec::with_capacity(items.len());
        for item in items {
            let name = item
                .get("name")
                .and_then(Value::as_str)
                .filter(|name| !name.trim().is_empty())
                .ok_or_else(|| {
                    extension(format!(
                        "outil MCP invalide renvoyé par « {} » : nom absent",
                        self.server_id
                    ))
                })?
                .to_string();
            let description = item
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let input_schema = item
                .get("inputSchema")
                .cloned()
                .unwrap_or_else(default_input_schema);
            tools.push(McpTool {
                name,
                description,
                input_schema,
            });
        }
        Ok(tools)
    }

    /// Appelle un outil et renvoie le contenu brut du champ `result`.
    pub async fn call_tool(&mut self, name: &str, arguments: Value) -> IhResult<Value> {
        if name.trim().is_empty() {
            return Err(extension("le nom de l'outil MCP est vide"));
        }
        self.request(
            "tools/call",
            Some(json!({
                "name": name,
                "arguments": arguments,
            })),
        )
        .await
    }

    /// Ferme proprement la session : ferme stdin, attend la sortie du
    /// processus, puis le tue s'il dépasse le délai.
    pub async fn shutdown(mut self) -> IhResult<()> {
        // Fermer stdin signale la fin de session aux serveurs bien élevés.
        self.stdin.take();

        if let Some(mut child) = self.child.take() {
            if self.dead {
                let _ = child.wait().await;
                return Ok(());
            }
            match tokio::time::timeout(self.timeout, child.wait()).await {
                Ok(Ok(_)) => Ok(()),
                Ok(Err(error)) => Err(extension(format!(
                    "attente du serveur MCP « {} » impossible : {error}",
                    self.server_id
                ))),
                Err(_) => {
                    let _ = child.start_kill();
                    let _ = child.wait().await;
                    tracing::debug!(server = %self.server_id, "serveur MCP tué à la fermeture");
                    Ok(())
                }
            }
        } else {
            Ok(())
        }
    }

    /// Indique si le processus enfant est encore vivant.
    pub fn is_alive(&mut self) -> bool {
        if self.dead {
            return false;
        }
        let Some(child) = self.child.as_mut() else {
            return false;
        };
        match child.try_wait() {
            Ok(None) => true,
            Ok(Some(_)) => {
                self.dead = true;
                false
            }
            Err(_) => false,
        }
    }

    /// Identifiant du processus enfant, utile aux diagnostics et aux tests.
    pub fn pid(&self) -> Option<u32> {
        self.child.as_ref().and_then(Child::id)
    }

    /// Délai maximal appliqué à chaque requête.
    pub fn request_timeout(&self) -> Duration {
        self.timeout
    }

    /// Modifie le délai maximal appliqué aux requêtes suivantes.
    pub fn set_request_timeout(&mut self, timeout: Duration) -> IhResult<()> {
        if timeout.is_zero() {
            return Err(extension(
                "le délai des requêtes MCP doit être supérieur à zéro",
            ));
        }
        self.timeout = timeout;
        Ok(())
    }

    async fn request(&mut self, method: &str, params: Option<Value>) -> IhResult<Value> {
        if self.dead {
            return Err(self.disconnected_error());
        }

        self.next_id += 1;
        let id = self.next_id;
        let mut payload = serde_json::Map::new();
        payload.insert("jsonrpc".into(), json!("2.0"));
        payload.insert("id".into(), json!(id));
        payload.insert("method".into(), json!(method));
        if let Some(params) = params {
            payload.insert("params".into(), params);
        }
        let line = serde_json::to_string(&Value::Object(payload))
            .map_err(|error| extension(format!("requête MCP non sérialisable : {error}")))?;

        let outcome = tokio::time::timeout(self.timeout, self.exchange(id, &line)).await;
        match outcome {
            Ok(Ok(result)) => Ok(result),
            Ok(Err(ExchangeError::Rpc(error))) => Err(error),
            Ok(Err(ExchangeError::Fatal(error))) => {
                self.terminate().await;
                Err(error)
            }
            Err(_) => {
                let error = extension(format!(
                    "délai de {} ms dépassé en attendant la réponse du serveur MCP « {} » à {method}",
                    self.timeout.as_millis(),
                    self.server_id
                ));
                self.terminate().await;
                Err(error)
            }
        }
    }

    async fn exchange(&mut self, id: u64, line: &str) -> Result<Value, ExchangeError> {
        self.write_line(line).await?;

        loop {
            let text = self.read_message().await?;
            let text = text.trim();
            if text.is_empty() {
                continue;
            }

            let message: Value = serde_json::from_str(text).map_err(|error| {
                ExchangeError::Fatal(extension(format!(
                    "réponse JSON-RPC invalide du serveur MCP « {} » : {error}",
                    self.server_id
                )))
            })?;

            if message.get("id").and_then(Value::as_u64) != Some(id) {
                tracing::trace!(
                    server = %self.server_id,
                    "message MCP ignoré (notification ou identifiant différent)"
                );
                continue;
            }

            if let Some(error) = message.get("error") {
                return Err(ExchangeError::Rpc(rpc_error(&self.server_id, error)));
            }
            return message.get("result").cloned().ok_or_else(|| {
                ExchangeError::Fatal(extension(format!(
                    "réponse JSON-RPC du serveur MCP « {} » sans « result »",
                    self.server_id
                )))
            });
        }
    }

    async fn write_line(&mut self, line: &str) -> Result<(), ExchangeError> {
        let mut framed = Vec::with_capacity(line.len() + 1);
        framed.extend_from_slice(line.as_bytes());
        framed.push(b'\n');

        let result = match self.stdin.as_mut() {
            Some(stdin) => write_all(stdin, &framed).await,
            None => Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "entrée standard du serveur MCP déjà fermée",
            )),
        };
        match result {
            Ok(()) => Ok(()),
            Err(error) => {
                self.dead = true;
                Err(ExchangeError::Fatal(extension(format!(
                    "écriture vers le serveur MCP « {} » impossible : {error}",
                    self.server_id
                ))))
            }
        }
    }

    async fn read_message(&mut self) -> Result<String, ExchangeError> {
        let mut buffer: Vec<u8> = Vec::new();

        loop {
            let available = self.stdout.fill_buf().await.map_err(|error| {
                ExchangeError::Fatal(extension(format!(
                    "lecture de la sortie du serveur MCP « {} » impossible : {error}",
                    self.server_id
                )))
            })?;

            if available.is_empty() {
                self.dead = true;
                let status = match self.child.as_mut() {
                    Some(child) => match child.try_wait() {
                        Ok(Some(status)) => format!("code de sortie {status}"),
                        Ok(None) => "processus encore actif".to_string(),
                        Err(error) => format!("statut indisponible : {error}"),
                    },
                    None => "processus absent".to_string(),
                };
                return Err(ExchangeError::Fatal(extension(format!(
                    "le serveur MCP « {} » a fermé sa sortie ({status})",
                    self.server_id
                ))));
            }

            let newline = available.iter().position(|byte| *byte == b'\n');
            let content_end = newline.unwrap_or(available.len());
            if buffer.len() + content_end > MAX_MESSAGE_BYTES {
                return Err(ExchangeError::Fatal(extension(format!(
                    "message MCP trop volumineux pour le serveur « {} » (limite de {MAX_MESSAGE_BYTES} octets)",
                    self.server_id
                ))));
            }
            buffer.extend_from_slice(&available[..content_end]);
            let consumed = newline.map_or(available.len(), |position| position + 1);
            self.stdout.consume(consumed);

            if newline.is_some() {
                break;
            }
        }

        if buffer.last() == Some(&b'\r') {
            buffer.pop();
        }
        String::from_utf8(buffer).map_err(|error| {
            ExchangeError::Fatal(extension(format!(
                "réponse MCP non UTF-8 du serveur « {} » : {error}",
                self.server_id
            )))
        })
    }

    async fn send_notification(&mut self, method: &str, params: Value) -> IhResult<()> {
        let line = serde_json::to_string(&json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        }))
        .map_err(|error| extension(format!("notification MCP non sérialisable : {error}")))?;

        let outcome = tokio::time::timeout(self.timeout, self.write_line(&line)).await;
        match outcome {
            Ok(Ok(())) => Ok(()),
            Ok(Err(ExchangeError::Fatal(error))) => {
                self.terminate().await;
                Err(error)
            }
            Ok(Err(ExchangeError::Rpc(error))) => Err(error),
            Err(_) => {
                let error = extension(format!(
                    "délai de {} ms dépassé en envoyant {method} au serveur MCP « {} »",
                    self.timeout.as_millis(),
                    self.server_id
                ));
                self.terminate().await;
                Err(error)
            }
        }
    }

    async fn terminate(&mut self) {
        self.dead = true;
        self.stdin.take();
        if let Some(mut child) = self.child.take() {
            let _ = child.start_kill();
            let _ = child.wait().await;
        }
    }

    fn disconnected_error(&self) -> IhError {
        extension(format!(
            "le client MCP « {} » n'est plus connecté",
            self.server_id
        ))
    }
}

impl Drop for McpClient {
    fn drop(&mut self) {
        self.stdin.take();
        let Some(mut child) = self.child.take() else {
            return;
        };
        let _ = child.start_kill();
        // Sans attente, le processus tué resterait zombie. On programme la
        // récupération du statut si un runtime Tokio est disponible ; sinon
        // `kill_on_drop` s'en charge côté pilote de signaux.
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                let _ = child.wait().await;
            });
        }
    }
}

async fn write_all(stdin: &mut ChildStdin, data: &[u8]) -> std::io::Result<()> {
    stdin.write_all(data).await?;
    stdin.flush().await
}

fn rpc_error(server_id: &str, error: &Value) -> IhError {
    let code = error
        .get("code")
        .and_then(Value::as_i64)
        .map_or_else(|| "?".to_string(), |code| code.to_string());
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("message absent");
    extension(format!(
        "erreur JSON-RPC du serveur MCP « {server_id} » (code {code}) : {message}"
    ))
}

fn extension(message: impl Into<String>) -> IhError {
    IhError::Extension(message.into())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::test_support::{write_file, TempDir};
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::time::Instant;

    /// Faux serveur MCP : lit une requête par ligne et répond en JSON-RPC.
    const FAKE_MCP_SCRIPT: &str = r#"#!/bin/sh
while IFS= read -r line; do
  id=$(printf '%s\n' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  if [ -z "$id" ]; then
    continue
  fi
  case "$line" in
    *'"method":"initialize"'*)
      printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":"2024-11-05","capabilities":{"tools":{}},"serverInfo":{"name":"fake-mcp","version":"0.0.1"}}}\n' "$id"
      ;;
    *'"method":"tools/list"'*)
      printf '{"jsonrpc":"2.0","id":%s,"result":{"tools":[{"name":"echo","description":"Répète le texte fourni","inputSchema":{"type":"object","properties":{"text":{"type":"string"}},"required":["text"]}},{"name":"fail","description":"Échoue toujours","inputSchema":{"type":"object"}}]}}\n' "$id"
      ;;
    *'"method":"tools/call"'*)
      case "$line" in
        *'"name":"fail"'*)
          printf '{"jsonrpc":"2.0","id":%s,"error":{"code":-32601,"message":"outil introuvable: fail"}}\n' "$id"
          ;;
        *'"name":"env"'*)
          printf '{"jsonrpc":"2.0","id":%s,"result":{"content":[{"type":"text","text":"%s"}]}}\n' "$id" "$FAKE_MCP_ENV"
          ;;
        *)
          printf '{"jsonrpc":"2.0","id":%s,"result":{"content":[{"type":"text","text":"echo"}],"isError":false}}\n' "$id"
          ;;
      esac
      ;;
    *)
      printf '{"jsonrpc":"2.0","id":%s,"error":{"code":-32601,"message":"méthode inconnue"}}\n' "$id"
      ;;
  esac
done
"#;

    /// Serveur muet : consomme les requêtes sans jamais répondre.
    const SILENT_SCRIPT: &str = r#"#!/bin/sh
while IFS= read -r line; do
  :
done
"#;

    /// Serveur qui crache une ligne de 9 Mio, au-delà de la limite de 8 Mio.
    const OVERSIZED_SCRIPT: &str = r#"#!/bin/sh
while IFS= read -r line; do
  head -c 9000000 /dev/zero | tr '\000' 'x'
  printf '\n'
done
"#;

    fn write_script(dir: &Path, name: &str, contents: &str) -> PathBuf {
        let path = dir.join(name);
        write_file(&path, contents);
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&path, permissions).unwrap();
        path
    }

    fn config(script: &Path) -> McpServerConfig {
        McpServerConfig {
            id: "fake".into(),
            command: script.to_string_lossy().into_owned(),
            args: Vec::new(),
            env: BTreeMap::new(),
        }
    }

    /// Lance le faux serveur en tolérant `ETXTBSY` : les tests tournent en
    /// parallèle et un `fork` concurrent peut hériter brièvement du descripteur
    /// d'écriture d'un script voisin, ce qui fait échouer `exec` de façon
    /// transitoire.
    async fn spawn_client(server: &McpServerConfig, timeout: Duration) -> McpClient {
        for attempt in 0..50 {
            match McpClient::spawn_with_timeout(server, timeout).await {
                Ok(client) => return client,
                Err(error) if attempt < 49 && error.to_string().contains("Text file busy") => {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                Err(error) => panic!("lancement du faux serveur MCP impossible : {error}"),
            }
        }
        unreachable!("la boucle de relance rend toujours la main")
    }

    #[cfg(target_os = "linux")]
    async fn wait_for_exit(pid: u32) {
        for _ in 0..150 {
            if !Path::new(&format!("/proc/{pid}")).exists() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("le processus {pid} existe encore : zombie ou non tué");
    }

    #[tokio::test]
    async fn initialisation_outils_et_appels() {
        let temp = TempDir::new("mcp-flow");
        let script = write_script(temp.path(), "fake-mcp.sh", FAKE_MCP_SCRIPT);
        let mut server = config(&script);
        server
            .env
            .insert("FAKE_MCP_ENV".into(), "valeur-test".into());

        let mut client = spawn_client(&server, Duration::from_secs(5)).await;
        assert!(client.is_alive());

        let info = client.initialize("ih-tests", "0.1.0").await.unwrap();
        assert_eq!(info["serverInfo"]["name"], "fake-mcp");
        assert!(info["capabilities"]["tools"].is_object());

        let tools = client.list_tools().await.unwrap();
        assert_eq!(tools.len(), 2);
        let echo = tools
            .iter()
            .find(|tool| tool.name == "echo")
            .expect("outil echo");
        assert_eq!(echo.description, "Répète le texte fourni");
        assert_eq!(echo.input_schema["type"], "object");
        assert_eq!(echo.input_schema["required"][0], "text");

        let result = client
            .call_tool("echo", json!({"text": "bonjour"}))
            .await
            .unwrap();
        assert_eq!(result["content"][0]["text"], "echo");

        let env = client.call_tool("env", json!({})).await.unwrap();
        assert_eq!(env["content"][0]["text"], "valeur-test");

        let error = client.call_tool("fail", json!({})).await.unwrap_err();
        assert!(matches!(&error, IhError::Extension(_)));
        let message = error.to_string();
        assert!(message.contains("-32601"), "code absent : {message}");
        assert!(
            message.contains("outil introuvable"),
            "message du serveur absent : {message}"
        );
        // Une erreur JSON-RPC ne doit pas tuer la session.
        assert!(client.is_alive());

        let pid = client.pid().expect("pid disponible");
        client.shutdown().await.unwrap();
        #[cfg(target_os = "linux")]
        wait_for_exit(pid).await;
    }

    #[tokio::test]
    async fn commande_inexistante_erreur_propre() {
        let temp = TempDir::new("mcp-absente");
        let server = McpServerConfig {
            id: "absente".into(),
            command: temp.join("commande-absente").to_string_lossy().into_owned(),
            args: Vec::new(),
            env: BTreeMap::new(),
        };
        let error = McpClient::spawn(&server).await.unwrap_err();
        assert!(matches!(&error, IhError::Extension(_)));
        assert!(
            error.to_string().contains("impossible de lancer"),
            "message imprécis : {error}"
        );
    }

    #[tokio::test]
    async fn serveur_qui_meurt_erreur_propre() {
        let server = McpServerConfig {
            id: "mort".into(),
            command: "/bin/sh".into(),
            args: vec!["-c".into(), "exit 0".into()],
            env: BTreeMap::new(),
        };
        let mut client = McpClient::spawn_with_timeout(&server, Duration::from_secs(2))
            .await
            .unwrap();
        let error = client.initialize("ih-tests", "0.1.0").await.unwrap_err();
        assert!(matches!(&error, IhError::Extension(_)));
        let message = error.to_string();
        // Selon le moment où le processus meurt, l'erreur est une écriture
        // interrompue ou une sortie déjà fermée : les deux sont explicites.
        assert!(
            message.contains("fermé sa sortie") || message.contains("écriture vers le serveur"),
            "message imprécis : {message}"
        );
        assert!(!client.is_alive());
        client.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn timeout_sur_serveur_muet() {
        let temp = TempDir::new("mcp-muet");
        let script = write_script(temp.path(), "muet.sh", SILENT_SCRIPT);
        let mut client = spawn_client(&config(&script), Duration::from_millis(250)).await;
        let pid = client.pid().expect("pid disponible");

        let started = Instant::now();
        let error = client.initialize("ih-tests", "0.1.0").await.unwrap_err();
        assert!(matches!(&error, IhError::Extension(_)));
        assert!(
            error.to_string().contains("délai"),
            "message imprécis : {error}"
        );
        assert!(started.elapsed() < Duration::from_secs(10));
        assert!(!client.is_alive());
        #[cfg(target_os = "linux")]
        wait_for_exit(pid).await;
        client.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn reponse_trop_volumineuse_est_refusee() {
        let temp = TempDir::new("mcp-volumineux");
        let script = write_script(temp.path(), "bavard.sh", OVERSIZED_SCRIPT);
        let mut client = spawn_client(&config(&script), Duration::from_secs(5)).await;

        let error = client.initialize("ih-tests", "0.1.0").await.unwrap_err();
        assert!(matches!(&error, IhError::Extension(_)));
        assert!(
            error.to_string().contains("volumineux"),
            "message imprécis : {error}"
        );
        assert!(!client.is_alive());
        client.shutdown().await.unwrap();
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn pas_de_zombie_apres_drop_sans_shutdown() {
        let temp = TempDir::new("mcp-drop");
        let script = write_script(temp.path(), "muet.sh", SILENT_SCRIPT);
        let client = spawn_client(&config(&script), Duration::from_secs(2)).await;
        let pid = client.pid().expect("pid disponible");

        drop(client);
        wait_for_exit(pid).await;
    }

    #[tokio::test]
    async fn delai_par_defaut_et_configurable() {
        let temp = TempDir::new("mcp-delai");
        let script = write_script(temp.path(), "fake-mcp.sh", FAKE_MCP_SCRIPT);
        let mut client = spawn_client(&config(&script), DEFAULT_REQUEST_TIMEOUT).await;
        assert_eq!(client.request_timeout(), DEFAULT_REQUEST_TIMEOUT);

        client
            .set_request_timeout(Duration::from_millis(500))
            .unwrap();
        assert_eq!(client.request_timeout(), Duration::from_millis(500));
        assert!(client.set_request_timeout(Duration::ZERO).is_err());

        client.shutdown().await.unwrap();
    }
}
