//! Moteur de conversation d'IH.
//!
//! Ce crate contient la logique du chat — historique, streaming, persistance,
//! annulation — et **aucun code Tauri**. L'interface ne fait qu'appeler
//! `ih-desktop`, qui délègue ici : la logique des fournisseurs n'est jamais
//! dupliquée côté React.
//!
//! # Garanties
//!
//! - le message utilisateur est persisté avant l'appel réseau ;
//! - chaque fragment reçu est transmis immédiatement au callback, dans l'ordre ;
//! - une annulation interrompt réellement le flux et conserve le texte partiel
//!   dans la base, marqué `cancelled` ;
//! - une erreur fournisseur ne crée pas de message assistant vide ;
//! - l'historique injecté est borné (`history_limit`, plafonné à 200 messages) ;
//! - la réponse est plafonnée en octets pour ne pas saturer la mémoire.

use ih_core::{IhError, IhResult, ThreadId};
use ih_providers::{ChatMessage, ChatRequest, Provider, Usage};
use ih_store::{NewMessage, Store};
use serde_json::json;
use std::time::Instant;
use tokio::sync::oneshot;

/// Limite dure de l'historique injecté dans une requête.
pub const MAX_HISTORY: usize = 200;
/// Limite dure de la réponse conservée (octets).
pub const MAX_REPLY_BYTES: usize = 2 * 1024 * 1024;
/// Titre par défaut d'une conversation nouvellement créée.
pub const DEFAULT_THREAD_TITLE: &str = "Nouvelle conversation";

/// Paramètres d'un tour de conversation.
#[derive(Debug, Clone)]
pub struct ChatTurn {
    pub thread_id: ThreadId,
    pub provider_id: String,
    pub model: String,
    pub prompt: String,
    pub system: Option<String>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    /// Nombre de messages antérieurs réinjectés (borné à [`MAX_HISTORY`]).
    pub history_limit: usize,
    /// Bloc de contexte projet déjà assemblé (fichiers réellement lus).
    /// Injecté dans le message système, jamais dans un message utilisateur.
    pub context: Option<String>,
}

impl ChatTurn {
    pub fn new(
        thread_id: ThreadId,
        provider_id: impl Into<String>,
        model: impl Into<String>,
        prompt: impl Into<String>,
    ) -> Self {
        Self {
            thread_id,
            provider_id: provider_id.into(),
            model: model.into(),
            prompt: prompt.into(),
            system: None,
            temperature: None,
            max_tokens: None,
            history_limit: 40,
            context: None,
        }
    }

    fn effective_history(&self) -> usize {
        self.history_limit.clamp(0, MAX_HISTORY)
    }
}

/// Résultat d'un tour, y compris lorsqu'il a été annulé.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ChatOutcome {
    pub user_message_id: String,
    pub assistant_message_id: String,
    pub text: String,
    pub usage: Usage,
    pub cancelled: bool,
    pub finish_reason: Option<String>,
    pub elapsed_ms: u64,
}

/// Émetteur d'annulation (conservé par l'appelant, par exemple une commande
/// Tauri qui le range dans un registre).
pub struct Canceller(Option<oneshot::Sender<()>>);

impl Canceller {
    /// Demande l'arrêt. Renvoie `false` si le tour est déjà terminé.
    pub fn cancel(mut self) -> bool {
        match self.0.take() {
            Some(sender) => sender.send(()).is_ok(),
            None => false,
        }
    }

    /// Vrai si le tour n'a pas encore été annulé ni consommé.
    pub fn armed(&self) -> bool {
        self.0.is_some()
    }
}

/// Récepteur d'annulation, consommé par [`run_turn`].
pub struct CancelToken(oneshot::Receiver<()>);

/// Crée un canal d'annulation lié à un tour.
pub fn cancel_channel() -> (Canceller, CancelToken) {
    let (tx, rx) = oneshot::channel();
    (Canceller(Some(tx)), CancelToken(rx))
}

/// Exécute un tour complet : persistance, streaming, annulation.
///
/// `on_delta` est appelé pour chaque fragment, avant la persistance finale.
pub async fn run_turn(
    provider: &dyn Provider,
    store: &Store,
    turn: &ChatTurn,
    cancel: CancelToken,
    on_delta: &mut (dyn for<'a> FnMut(&'a str) + Send),
) -> IhResult<ChatOutcome> {
    if turn.prompt.trim().is_empty() {
        return Err(IhError::Invalid("message vide".into()));
    }
    let thread = store
        .get_thread(&turn.thread_id)?
        .ok_or_else(|| IhError::NotFound(format!("conversation {}", turn.thread_id)))?;

    // L'historique est lu AVANT l'ajout du nouveau message.
    let history = store.list_messages(&turn.thread_id, MAX_HISTORY as u32, 0)?;
    let start = history.len().saturating_sub(turn.effective_history());
    let mut messages: Vec<ChatMessage> = history[start..]
        .iter()
        .filter(|m| m.role == "user" || m.role == "assistant")
        .map(|m| ChatMessage {
            role: m.role.clone(),
            content: m.content.clone(),
        })
        .collect();

    let user_message = store.append_message(&NewMessage {
        thread_id: turn.thread_id.clone(),
        role: "user".into(),
        content: turn.prompt.clone(),
        metadata: Some(json!({
            "provider": turn.provider_id,
            "model": turn.model,
        })),
    })?;

    messages.push(ChatMessage {
        role: "user".into(),
        content: turn.prompt.clone(),
    });

    let request = ChatRequest {
        model: turn.model.clone(),
        messages,
        system: merge_system(&turn.system, &turn.context),
        temperature: turn.temperature,
        max_tokens: turn.max_tokens,
    };

    let started = Instant::now();
    let mut collected = String::new();
    let mut cancelled = false;
    let mut usage = Usage::default();
    let mut finish_reason: Option<String> = None;
    let mut truncation_notified = false;

    // Le futur et le callback sont confinés dans ce bloc : la réponse est
    // traitée après leur destruction, ce qui libère l'emprunt sur `collected`.
    let streamed: Option<IhResult<ih_providers::ChatResponse>> = {
        let mut forward = |delta: &str| {
            if collected.len() < MAX_REPLY_BYTES {
                collected.push_str(delta);
            } else if !truncation_notified {
                truncation_notified = true;
                on_delta("\n[réponse tronquée : limite de taille atteinte]");
            }
            on_delta(delta);
        };
        let mut stream = Box::pin(provider.chat_stream(request, &mut forward));
        let mut cancel = Box::pin(cancel.0);

        tokio::select! {
            result = &mut stream => Some(result),
            _ = &mut cancel => None,
        }
    };

    match streamed {
        None => cancelled = true,
        Some(Err(error)) => {
            // Aucun message assistant n'est créé en cas d'erreur : la
            // conversation garde uniquement la demande.
            return Err(error);
        }
        Some(Ok(response)) => {
            // Le texte complet fait foi s'il diffère des fragments.
            if !response.text.is_empty() {
                collected = response.text;
            }
            usage = response.usage;
            finish_reason = response.finish_reason;
        }
    }

    let assistant_message = store.append_message(&NewMessage {
        thread_id: turn.thread_id.clone(),
        role: "assistant".into(),
        content: collected.clone(),
        metadata: Some(json!({
            "provider": turn.provider_id,
            "model": turn.model,
            "cancelled": cancelled,
            "usage": {
                "input_tokens": usage.input_tokens,
                "output_tokens": usage.output_tokens,
            },
            "finish_reason": finish_reason,
            "elapsed_ms": started.elapsed().as_millis() as u64,
        })),
    })?;

    if thread.title == DEFAULT_THREAD_TITLE || thread.title.trim().is_empty() {
        let title = derive_title(&turn.prompt);
        store.rename_thread(&turn.thread_id, &title)?;
    }

    Ok(ChatOutcome {
        user_message_id: user_message.id.into_string(),
        assistant_message_id: assistant_message.id.into_string(),
        text: collected,
        usage,
        cancelled,
        finish_reason,
        elapsed_ms: started.elapsed().as_millis() as u64,
    })
}

/// Concatène le message système et le contexte projet, sans perdre l'un ou l'autre.
fn merge_system(system: &Option<String>, context: &Option<String>) -> Option<String> {
    match (system.as_deref(), context.as_deref()) {
        (Some(sys), Some(ctx)) if !sys.trim().is_empty() && !ctx.trim().is_empty() => {
            Some(format!("{sys}\n\n{ctx}"))
        }
        (Some(sys), _) if !sys.trim().is_empty() => Some(sys.to_string()),
        (_, Some(ctx)) if !ctx.trim().is_empty() => Some(ctx.to_string()),
        _ => None,
    }
}

/// Titre court dérivé du premier message (une seule ligne, 60 caractères max).
pub fn derive_title(prompt: &str) -> String {
    let line = prompt
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or(DEFAULT_THREAD_TITLE);
    let mut title: String = line.chars().take(60).collect();
    if line.chars().count() > 60 {
        title.push('…');
    }
    if title.trim().is_empty() {
        DEFAULT_THREAD_TITLE.to_string()
    } else {
        title
    }
}

/// Recherche de conversations par titre ou contenu, sans requête SQL côté
/// appelant (utilisé par l'interface).
pub fn search_threads(
    store: &Store,
    project_id: &ih_core::ProjectId,
    query: &str,
    limit: usize,
) -> IhResult<Vec<ih_store::Thread>> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return store.list_threads(project_id);
    }
    let mut matches = Vec::new();
    for thread in store.list_threads(project_id)? {
        let title_match = thread.title.to_lowercase().contains(&needle);
        let message_match = store
            .list_messages(&thread.id, 200, 0)?
            .iter()
            .any(|m| m.content.to_lowercase().contains(&needle));
        if title_match || message_match {
            matches.push(thread);
        }
        if matches.len() >= limit {
            break;
        }
    }
    Ok(matches)
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use ih_core::ProjectId;
    use ih_providers::{ChatResponse, HealthReport, ModelEntry};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    /// Fournisseur factice : rejoue des fragments, enregistre les requêtes.
    struct FakeProvider {
        chunks: Vec<String>,
        fail_at: Option<usize>,
        requests: Mutex<Vec<ChatRequest>>,
        step: Duration,
    }

    impl FakeProvider {
        fn new(chunks: &[&str]) -> Self {
            Self {
                chunks: chunks.iter().map(|c| c.to_string()).collect(),
                fail_at: None,
                requests: Mutex::new(Vec::new()),
                step: Duration::from_millis(15),
            }
        }

        fn failing() -> Self {
            Self {
                chunks: vec!["jamais".into()],
                fail_at: Some(0),
                requests: Mutex::new(Vec::new()),
                step: Duration::from_millis(1),
            }
        }

        fn last_request(&self) -> ChatRequest {
            self.requests
                .lock()
                .unwrap()
                .last()
                .cloned()
                .expect("requête")
        }
    }

    #[async_trait]
    impl Provider for FakeProvider {
        fn id(&self) -> &str {
            "fake"
        }

        fn kind(&self) -> ih_core::config::ProviderKind {
            ih_core::config::ProviderKind::OpenaiCompatible
        }

        async fn list_models(&self) -> IhResult<Vec<ModelEntry>> {
            Ok(vec![ModelEntry {
                id: "fake-1".into(),
                ..Default::default()
            }])
        }

        async fn health(&self) -> IhResult<HealthReport> {
            Ok(HealthReport {
                ok: true,
                latency_ms: 1,
                detail: "fake".into(),
            })
        }

        async fn chat(&self, request: ChatRequest) -> IhResult<ChatResponse> {
            self.requests.lock().unwrap().push(request);
            Ok(ChatResponse {
                text: self.chunks.concat(),
                usage: Usage::default(),
                finish_reason: Some("stop".into()),
                model: "fake-1".into(),
            })
        }

        async fn chat_stream(
            &self,
            request: ChatRequest,
            on_delta: &mut (dyn for<'a> FnMut(&'a str) + Send),
        ) -> IhResult<ChatResponse> {
            self.requests.lock().unwrap().push(request);
            if let Some(index) = self.fail_at {
                if index == 0 {
                    return Err(IhError::Provider("échec simulé du fournisseur".into()));
                }
            }
            let mut text = String::new();
            for chunk in &self.chunks {
                tokio::time::sleep(self.step).await;
                text.push_str(chunk);
                on_delta(chunk);
            }
            Ok(ChatResponse {
                text,
                usage: Usage {
                    input_tokens: Some(11),
                    output_tokens: Some(7),
                },
                finish_reason: Some("stop".into()),
                model: "fake-1".into(),
            })
        }
    }

    fn fixture() -> (Store, ProjectId, ThreadId) {
        let store = Store::open_in_memory().unwrap();
        let project = ih_store::Project {
            id: ProjectId::new(),
            path: "/tmp/ih-chat-test".into(),
            name: "test".into(),
            created_at: chrono::Utc::now(),
            last_opened_at: chrono::Utc::now(),
            trusted: true,
        };
        store.upsert_project(&project).unwrap();
        let thread = store
            .create_thread(&project.id, DEFAULT_THREAD_TITLE, None)
            .unwrap();
        (store, project.id, thread.id)
    }

    #[tokio::test]
    async fn streaming_persiste_les_deux_messages_et_l_usage() {
        let (store, _project, thread_id) = fixture();
        let provider = FakeProvider::new(&["Bonjour", ", ", "monde"]);
        let mut deltas = Vec::new();
        let mut sink = |delta: &str| deltas.push(delta.to_string());

        let outcome = run_turn(
            &provider,
            &store,
            &ChatTurn::new(thread_id.clone(), "fake", "fake-1", "salut"),
            cancel_channel().1,
            &mut sink,
        )
        .await
        .unwrap();

        assert_eq!(deltas, vec!["Bonjour", ", ", "monde"]);
        assert_eq!(outcome.text, "Bonjour, monde");
        assert!(!outcome.cancelled);

        let messages = store.list_messages(&thread_id, 10, 0).unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[0].content, "salut");
        assert_eq!(messages[1].role, "assistant");
        assert_eq!(messages[1].content, "Bonjour, monde");

        let metadata = messages[1].metadata.as_ref().unwrap();
        assert_eq!(metadata["usage"]["output_tokens"], 7);
        assert_eq!(metadata["cancelled"], false);
    }

    #[tokio::test]
    async fn annulation_conserve_le_texte_partiel() {
        let (store, _project, thread_id) = fixture();
        let provider = Arc::new(FakeProvider {
            chunks: vec!["a".into(), "b".into(), "c".into(), "d".into()],
            fail_at: None,
            requests: Mutex::new(Vec::new()),
            step: Duration::from_millis(60),
        });
        let (canceller, token) = cancel_channel();
        let store = Arc::new(store);
        let thread = thread_id.clone();

        let handle = {
            let provider = provider.clone();
            let store = store.clone();
            tokio::spawn(async move {
                let mut sink = |_: &str| {};
                run_turn(
                    provider.as_ref(),
                    store.as_ref(),
                    &ChatTurn::new(thread, "fake", "fake-1", "long"),
                    token,
                    &mut sink,
                )
                .await
            })
        };

        tokio::time::sleep(Duration::from_millis(80)).await;
        assert!(canceller.cancel(), "l'annulation doit être acceptée");
        let outcome = handle.await.unwrap().unwrap();
        assert!(outcome.cancelled);
        assert!(!outcome.text.is_empty(), "texte partiel attendu");
        assert!(outcome.text.len() < 4 * 3, "le flux doit être interrompu");

        let messages = store.list_messages(&thread_id, 10, 0).unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1].role, "assistant");
        assert_eq!(messages[1].content, outcome.text);
        assert_eq!(messages[1].metadata.as_ref().unwrap()["cancelled"], true);
    }

    #[tokio::test]
    async fn erreur_fournisseur_ne_cree_pas_de_message_assistant() {
        let (store, _project, thread_id) = fixture();
        let provider = FakeProvider::failing();
        let mut sink = |_: &str| {};
        let error = run_turn(
            &provider,
            &store,
            &ChatTurn::new(thread_id.clone(), "fake", "fake-1", "question"),
            cancel_channel().1,
            &mut sink,
        )
        .await
        .unwrap_err();
        assert!(matches!(error, IhError::Provider(_)));

        let messages = store.list_messages(&thread_id, 10, 0).unwrap();
        assert_eq!(messages.len(), 1, "seule la demande est conservée");
        assert_eq!(messages[0].role, "user");
    }

    #[tokio::test]
    async fn historique_reinjecte_dans_l_ordre() {
        let (store, _project, thread_id) = fixture();
        let provider = FakeProvider::new(&["ok"]);
        let mut sink = |_: &str| {};

        run_turn(
            &provider,
            &store,
            &ChatTurn::new(thread_id.clone(), "fake", "fake-1", "premier"),
            cancel_channel().1,
            &mut sink,
        )
        .await
        .unwrap();
        run_turn(
            &provider,
            &store,
            &ChatTurn::new(thread_id.clone(), "fake", "fake-1", "deuxième"),
            cancel_channel().1,
            &mut sink,
        )
        .await
        .unwrap();

        let request = provider.last_request();
        let roles: Vec<_> = request.messages.iter().map(|m| m.role.as_str()).collect();
        assert_eq!(roles, vec!["user", "assistant", "user"]);
        assert_eq!(request.messages[0].content, "premier");
        assert_eq!(request.messages[2].content, "deuxième");
    }

    #[tokio::test]
    async fn historique_borne_par_la_limite() {
        let (store, _project, thread_id) = fixture();
        let provider = FakeProvider::new(&["ok"]);
        let mut sink = |_: &str| {};
        for index in 0..3 {
            run_turn(
                &provider,
                &store,
                &ChatTurn::new(thread_id.clone(), "fake", "fake-1", format!("tour {index}")),
                cancel_channel().1,
                &mut sink,
            )
            .await
            .unwrap();
        }

        let mut turn = ChatTurn::new(thread_id.clone(), "fake", "fake-1", "dernier");
        turn.history_limit = 2;
        run_turn(&provider, &store, &turn, cancel_channel().1, &mut sink)
            .await
            .unwrap();

        let request = provider.last_request();
        assert_eq!(
            request.messages.len(),
            3,
            "2 d'historique + le nouveau message"
        );
        assert_eq!(request.messages.last().unwrap().content, "dernier");
    }

    #[tokio::test]
    async fn conversation_inconnue_et_message_vide_rejetes() {
        let (store, _project, _thread_id) = fixture();
        let provider = FakeProvider::new(&["ok"]);
        let mut sink = |_: &str| {};

        let missing = ThreadId::new();
        let error = run_turn(
            &provider,
            &store,
            &ChatTurn::new(missing, "fake", "fake-1", "salut"),
            cancel_channel().1,
            &mut sink,
        )
        .await
        .unwrap_err();
        assert!(matches!(error, IhError::NotFound(_)));

        let (_project_id, thread_id) = {
            let project = store.list_projects().unwrap().pop().unwrap();
            let thread = store
                .create_thread(&project.id, DEFAULT_THREAD_TITLE, None)
                .unwrap();
            (project.id, thread.id)
        };
        let error = run_turn(
            &provider,
            &store,
            &ChatTurn::new(thread_id, "fake", "fake-1", "   "),
            cancel_channel().1,
            &mut sink,
        )
        .await
        .unwrap_err();
        assert!(matches!(error, IhError::Invalid(_)));
    }

    #[tokio::test]
    async fn titre_derive_du_premier_message() {
        let (store, _project, thread_id) = fixture();
        let provider = FakeProvider::new(&["ok"]);
        let mut sink = |_: &str| {};
        run_turn(
            &provider,
            &store,
            &ChatTurn::new(
                thread_id.clone(),
                "fake",
                "fake-1",
                "Ajoute un système de chat à IH avec streaming.",
            ),
            cancel_channel().1,
            &mut sink,
        )
        .await
        .unwrap();

        let thread = store.get_thread(&thread_id).unwrap().unwrap();
        assert_eq!(
            thread.title,
            "Ajoute un système de chat à IH avec streaming."
        );
    }

    #[tokio::test]
    async fn contexte_projet_injecte_dans_le_message_systeme() {
        let (store, _project, thread_id) = fixture();
        let provider = FakeProvider::new(&["ok"]);
        let mut sink = |_: &str| {};
        let mut turn = ChatTurn::new(thread_id.clone(), "fake", "fake-1", "analyse ce fichier");
        turn.system = Some("Tu es un assistant de développement.".into());
        turn.context = Some(
            "Contexte projet (fichiers réellement lus) :\n--- src/main.rs ---\nfn main() {}".into(),
        );
        run_turn(&provider, &store, &turn, cancel_channel().1, &mut sink)
            .await
            .unwrap();

        let request = provider.last_request();
        let system = request.system.expect("message système");
        assert!(
            system.contains("assistant de développement"),
            "system: {system}"
        );
        assert!(system.contains("src/main.rs"), "contexte absent: {system}");
        // Le contexte ne pollue pas le message utilisateur.
        assert_eq!(
            request.messages.last().unwrap().content,
            "analyse ce fichier"
        );
    }

    #[tokio::test]
    async fn sans_contexte_le_comportement_est_inchange() {
        let (store, _project, thread_id) = fixture();
        let provider = FakeProvider::new(&["ok"]);
        let mut sink = |_: &str| {};
        run_turn(
            &provider,
            &store,
            &ChatTurn::new(thread_id, "fake", "fake-1", "salut"),
            cancel_channel().1,
            &mut sink,
        )
        .await
        .unwrap();
        assert!(provider.last_request().system.is_none());
    }

    #[test]
    fn derive_title_borne_et_une_ligne() {
        assert_eq!(derive_title("  "), DEFAULT_THREAD_TITLE);
        assert_eq!(derive_title("ligne 1\nligne 2"), "ligne 1");
        let long = "a".repeat(100);
        let title = derive_title(&long);
        assert_eq!(title.chars().count(), 61, "60 caractères + ellipse");
        assert!(title.ends_with('…'));
    }

    #[test]
    fn recherche_par_titre_et_contenu() {
        let (store, project_id, thread_id) = fixture();
        let provider = FakeProvider::new(&["réponse au sujet du streaming"]);
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let mut sink = |_: &str| {};
            run_turn(
                &provider,
                &store,
                &ChatTurn::new(thread_id.clone(), "fake", "fake-1", "parle du chat"),
                cancel_channel().1,
                &mut sink,
            )
            .await
            .unwrap();
        });

        assert_eq!(
            search_threads(&store, &project_id, "chat", 10)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            search_threads(&store, &project_id, "streaming", 10)
                .unwrap()
                .len(),
            1,
            "la recherche couvre aussi les messages"
        );
        assert!(search_threads(&store, &project_id, "inexistant", 10)
            .unwrap()
            .is_empty());
    }
}
