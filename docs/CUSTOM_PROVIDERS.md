# Fournisseurs personnalisés et endpoints API

IH permet d'ajouter n'importe quel endpoint **compatible avec l'un des quatre
protocoles** supportés, sans recompiler l'application : LM Studio, vLLM, Groq,
OpenRouter, une passerelle privée, un serveur d'entreprise…

## Principe

Un fournisseur personnalisé est un `ProviderConfig` comme les autres : il
utilise les mêmes clients (`ih-providers`), donc le chat, le streaming SSE /
NDJSON, l'annulation et la persistance fonctionnent sans code spécifique.

> **Une URL ne suffit pas.** Le protocole choisi doit correspondre à celui
> réellement exposé par le serveur. Un serveur qui parle « Anthropic Messages »
> ne répondra pas à un chemin OpenAI, et inversement.

## Protocoles

| Protocole | Chemin par défaut (si `endpoint_path` est vide) |
|---|---|
| `openai_compatible` | `{base_url}/chat/completions` (SSE), modèles `{base_url}/models` |
| `anthropic` | `{base_url}/v1/messages` (SSE), modèles `{base_url}/v1/models` |
| `google` | `{base_url}/v1beta/models/{model}:generateContent` (SSE via `?alt=sse`) |
| `ollama` | `{base_url}/api/chat` (NDJSON), modèles `{base_url}/api/tags` |

### Chemins personnalisés

`endpoint_path` remplace **le chemin complet** de l'endpoint de chat, en
conservant l'origine de `base_url` :

```
base_url      = https://gateway.interne
endpoint_path = /llm/v1/chat/completions
→ https://gateway.interne/llm/v1/chat/completions
```

Le chemin doit commencer par `/`, ne pas contenir `..` ni espace.

## Champs

| Champ | Rôle |
|---|---|
| `id` | identifiant slug (`a-z`, `0-9`, `-`, `_`), unique |
| `label` | nom affiché |
| `kind` | protocole (voir tableau) |
| `base_url` | origine (et chemin de base si `endpoint_path` est vide) |
| `endpoint_path` | chemin complet du chat (optionnel) |
| `headers` | en-têtes HTTP additionnels (`X-Tenant: acme`, `HTTP-Referer: …`) |
| `timeout_secs` | délai maximal par requête |
| `streaming` | `false` ⇒ réponse complète en un bloc, diffusée en un seul delta |
| `temperature`, `max_tokens` | paramètres de génération appliqués si renseignés |
| `default_model` | modèle présélectionné dans le chat |
| `models` | liste connue (découverte ou saisie manuelle) |

## Modèles

- **Découverte** : bouton « Découvrir » (`models_list`) ; si l'API ne propose
  pas de découverte, l'erreur réelle est affichée et l'ajout manuel reste
  disponible.
- **Ajout manuel** : identifiant conservé **exactement** comme saisi.
- Les identifiants renvoyés par le fournisseur ne sont jamais normalisés.

## Test de connexion

`provider_test` renvoie un statut exploitable :

| Statut | Signification |
|---|---|
| `ok` | endpoint joignable, authentification acceptée, modèles listés |
| `auth_failed` | 401/403 ou clé absente/invalide |
| `unreachable` | connexion refusée, DNS, hôte injoignable |
| `unsupported_protocol` | 404/405 ou réponse non conforme au protocole |
| `model_unavailable` | connexion OK mais découverte des modèles impossible |
| `timeout` | délai dépassé |
| `error` | autre erreur, détail brut fourni |

La clé d'API n'apparaît jamais dans le résultat.

## Sécurité

- **Clés** : trousseau du système uniquement (`has_secret` dans la config, la
  valeur jamais). Aucune clé en SQLite, dans les logs ou dans un événement.
- **En-têtes** : `Authorization`, `Host`, `Content-Length`, `Transfer-Encoding`
  et `Connection` sont **refusés** (la clé passe par le trousseau) ; les noms
  et valeurs sont validés (pas de retour à la ligne → pas d'injection).
- **URL** : HTTPS obligatoire, sauf hôte local (localhost, `127.0.0.0/8`, `::1`,
  réseaux privés `10/8`, `172.16/12`, `192.168/16`, `*.local`). Les serveurs
  locaux restent donc utilisables.
- **Redirections** : jamais suivies (une redirection pourrait envoyer la clé
  d'API vers un autre hôte) ; un 3xx est remonté comme erreur.
- **Timeouts** : appliqués à chaque requête.

## Exemples

```text
LM Studio     base_url = http://127.0.0.1:1234/v1      kind = openai_compatible
vLLM          base_url = http://127.0.0.1:8000/v1      kind = openai_compatible
Groq          base_url = https://api.groq.com/openai/v1 kind = openai_compatible
OpenRouter    base_url = https://openrouter.ai/api/v1   kind = openai_compatible
Passerelle    base_url = https://llm.interne
              endpoint_path = /v1/chat/completions
              headers = { "X-Tenant": "acme" }
```

## Dépannage

| Symptôme | Cause probable |
|---|---|
| `unsupported_protocol` | mauvais protocole, ou `endpoint_path` erroné |
| `auth_failed` | clé absente (Ollama n'en veut pas), ou en-tête d'auth exotique |
| `unreachable` | service arrêté, port erroné, pare-feu |
| `model_unavailable` | serveur sans `/models` : saisir le modèle à la main |
| Réponse vide | modèle inconnu du serveur : vérifier l'identifiant exact |

## Limites

- Quatre protocoles seulement ; ajouter un protocole demande un adaptateur Rust.
- Pas de découverte des capacités (vision, outils) au-delà de ce que l'API
  expose ; les champs restent vides plutôt qu'inventés.
- Le streaming dépend du support serveur ; `streaming = false` force une
  réponse complète.

## Validation effectuée

| Vérification | Où | Résultat |
|---|---|---|
| Validation de configuration (hôtes locaux, chemins, en-têtes, bornes) | tests `ih-core` | ✅ |
| Endpoint personnalisé + en-têtes transmis | tests d'intégration `ih-providers` (serveur HTTP local) | ✅ |
| Streaming désactivé (un seul delta) | idem | ✅ |
| Redirection 302 non suivie | idem | ✅ |
| Classification des erreurs de connexion | test unitaire `ih-providers` | ✅ |
| Formulaire, en-têtes, statuts | 21 tests frontend | ✅ |
| Création/modification via l'interface (GUI) | — | ❌ non exécuté : le lancement graphique de validation s'est bloqué dans l'environnement Xvfb + trousseau ; le contrat IPC est couvert par les tests mais **aucun scénario GUI complet n'a été joué** |
| Endpoint cloud réel (OpenAI, Groq…) | — | ❌ aucune clé disponible |

Le serveur local utilisé pour les tests d'intégration attend
`Authorization: Bearer <clé>` **et** `X-Tenant: acme`, ce qui vérifie à la fois
l'authentification et les en-têtes personnalisés.
