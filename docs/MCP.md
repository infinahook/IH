# MCP dans IH — intégration native (0.6.1)

IH se connecte directement à des serveurs **Model Context Protocol** (MCP)
distants : découverte de leurs outils, catalogue local, activation explicite,
appels contrôlés par le système d'approbation d'IH. Aucun serveur MCP ne peut
contourner les permissions de l'application.

> **Statut honnête.** Les transports réseau (Streamable HTTP et SSE historique)
> sont implémentés et **testés contre un vrai serveur MCP local** (TCP 127.0.0.1,
> JSON-RPC et flux SSE réels). L'invocation est contrôlée par l'interface.
> L'utilisation des outils par les **modèles de chat** (tool use) et
> l'injection directe dans les **CLI d'agents** ne sont pas encore câblées ;
> voir [Limites](#limites-connues).

## Architecture

```
src/ (React) ── IPC typé ──▶ ih-desktop (Tauri 2)
                              │ commands/mcp.rs  ── point d'entrée unique
                              │   validate argumentés ─ approbation ─ audit
                              │ state.mcp_sessions ── sessions partagées
                              ▼
                          ih-mcp (nouvelle crate)
                              │ Streamable HTTP ── rmcp (SDK officiel Apache-2.0)
                              │ SSE historique   ── client maison, borné
                              │ sécurité         ── ih-security::net (SSRF,
                              │                    classification, empreintes)
                              ▼
                          SQLite (ih-store, migration v4)
                              │ mcp_servers · mcp_tools · mcp_calls
                              ▼
                          Trousseau (secrets) : mcp:<id>
```

### Crates

| Crate | Rôle |
|---|---|
| `ih-mcp` | client MCP : config, transports, session unifiée, validation de schéma et d'arguments, URL épinglée |
| `ih-security` | politique réseau anti-SSRF, classification des risques MCP, empreintes d'approbation, compte trousseau |
| `ih-store` | persistance : serveurs, catalogue, historique (migration v4) |
| `ih-desktop` | commandes IPC, sessions, événements, audit |

### Dépendance ajoutée

`rmcp` **3.11** (crates.io, dépôt `modelcontextprotocol/rust-sdk`, licence
**Apache-2.0**, MSRV 1.88). Le protocole (négociation, sessions
`Mcp-Session-Id`, SSE de réponse, reconnexions, expiration de session) est
délégué au SDK ; IH n'ajoute que la politique de sécurité.

## Transports

### Streamable HTTP (spécification 2025+) — implémenté

Géré par `rmcp::transport::streamable_http_client` :
- handshake `initialize` / `notifications/initialized` ;
- sessions et en-têtes `Mcp-Session-Id`, expiration et re-création de session ;
- flux SSE de réponse et reconnexions avec repli exponentiel ;
- bornes IH : réponse JSON ≤ 8 Mio, événement SSE ≤ 8 Mio, délai par requête
  configurable (défaut 30 s).

### SSE historique (HTTP+SSE, spécification 2024-11-05) — implémenté

Le SDK officiel a **retiré** ce transport (« remove SSE transport support »).
IH l'implémente donc directement (`crates/ih-mcp/src/sse.rs`), comme couche de
transport seulement :
- `GET` du flux, attente bornée de l'événement `endpoint` ;
- `POST` des requêtes JSON-RPC, réponses reçues en événements `message` ;
- taille maximale par événement, délai en envoi **et** en attente ;
- reconnexions bornées (5 tentatives, repli exponentiel) puis abandon explicite ;
  les requêtes en vol échouent proprement et un nouveau handshake est rejoué
  sur la nouvelle session.

### stdio local — hors de cette livraison

Les serveurs stdio existants (`ih-extensions::mcp`, page Plugins) continuent de
fonctionner. Leur migration vers `ih-mcp` (transport stdio unifié,
classification et approbations identiques) est prévue dans une version
ultérieure ; aucune annonce contraire n'est faite dans l'interface.

## Installation et configuration

**Paramètres → Serveurs MCP** (section « MCP » de la barre latérale).

1. **Nom** — libellé local.
2. **URL** — `https://` vers Internet ; `http://` accepté **uniquement** pour
   localhost et les réseaux privés. Aucun identifiant dans l'URL, pas de
   fragment.
3. **Transport** — Streamable HTTP (défaut) ou SSE (historique).
4. **Délai** — secondes par requête (défaut 30).
5. **Jeton** — stocké dans le **trousseau**, jamais en base ni dans les
   journaux ; laissé vide lors d'une édition, il reste inchangé.
6. **En-têtes HTTP (JSON)** — non secrets ; `Authorization`, `Cookie` et les
   en-têtes de cadrage sont refusés.
7. **Activer** / **Pré-autoriser les lectures** — voir [Permissions](#permissions).

Actions : Connecter · Déconnecter · Tester · Rafraîchir les outils · Activer /
Désactiver · Modifier · Supprimer (confirmation). Statuts : Déconnecté,
Connexion en cours, Connecté, Erreur (avec le message).

### Exemple de serveur MCP compatible

Serveurs MCP publics et outils de développement : serveurs `filesystem`,
`fetch`, `github`, `sqlite` (stdio, à exposer via un pont HTTP/SSE de votre
choix), ou tout serveur HTTP respectant la spécification. Exemple minimal
testable localement : le serveur de test d'`ih-mcp` (voir
`crates/ih-mcp/tests/support/mod.rs`) expose `/mcp` (Streamable HTTP) et
`/sse` (SSE historique) en JSON-RPC réel.

## Authentification

- **Jeton bearer** : champ dédié, rangé au trousseau
  (`mcp:<id>`), envoyé en `Authorization: Bearer …` sur les deux transports.
- **En-têtes personnalisés** pour les autres schémas (API keys en en-tête),
  filtrage strict des noms/valeurs, `Authorization`/`Cookie` réservés.
- **OAuth 2.1 : hors périmètre de cette version.** Le SDK `rmcp` embarque une
  couche OAuth, mais l'interface d'autorisation (enregistrement de client,
  redirections locales, rotation de jeton) n'est pas encore intégrée. Un faux
  OAuth simplifié n'est **pas** implémenté : cette limite est assumée.

## Permissions

1. **Un outil découvert n'est jamais autorisé par défaut** : `enabled = 0` en
   base tant que l'utilisateur ne l'active pas individuellement.
2. **Classification locale** (jamais fournie par le serveur comme permission) :
   lecture (bas), écriture (moyen), exécution (élevé), destructif (critique),
   réseau externe (moyen). Les annotations du serveur (`readOnlyHint`,
   `destructiveHint`, `openWorldHint`) ne sont que des signaux de départ.
3. **Approbation interactive à usage unique**, liée par SHA-256 à l'empreinte
   canonique des paramètres exacts `{serveur, outil, arguments}` : une
   approbation refusée, expirée, déjà consommée ou demandée pour d'autres
   paramètres est rejetée.
4. **Pré-autorisation explicite** : pour les automatisations sans interaction,
   seule une **lecture** (risque bas) d'un serveur dont l'option
   « pré-autoriser les lectures » est activée peut s'exécuter sans dialogue.
   Tout le reste exige une approbation : un workflow qui tenterait une
   écriture sans approbation échoue en `approval_required` et laisse une trace.
5. **Journal d'audit** : chaque décision (`approved`, `auto`, refus) est
   enregistrée dans la table `audit` d'IH.

## Modèle de sécurité

- **HTTPS obligatoire** vers Internet ; HTTP seulement vers loopback/réseaux
  privés (politique par défaut d'IH).
- **Anti-SSRF** : validation statique de l'URL (schéma, identifiants, hôte,
  port), résolution DNS, **revalidation de chaque adresse** puis **épinglage**
  des IP validées dans le client HTTP — le DNS rebinding entre résolution et
  connexion est neutralisé.
- **Destinations interdites** : link-local `169.254.0.0/16` (dont l'endpoint de
  métadonnées cloud `169.254.169.254`), `0.0.0.0/8`, multicast, broadcast,
  CGNAT `100.64.0.0/10`, plages réservées/benchmark/documentation, `::`,
  `fe80::/10`, `ff00::/8`, équivalents IPv4-mappés.
- **Redirections HTTP non suivies** (`redirect::Policy::none`) et **proxys
  d'environnement ignorés** (`no_proxy`) : aucun contournement par redirection
  et aucune fuite de jeton vers un autre hôte.
- **Secrets** : uniquement dans le trousseau ; jamais en SQLite, jamais dans
  l'interface React, jamais dans les journaux, les erreurs ou les événements.
- **Tailles bornées** : requêtes/réponses JSON ≤ 8 Mio, événements SSE ≤ 8 Mio,
  schémas ≤ 512 Kio (profondeur et nœuds bornés), arguments historisés ≤ 8 Kio,
  texte de résultat ≤ 256 Kio pour l'affichage (troncature signalée).
- **Données du serveur non fiables** : schémas invalides exclus du catalogue ;
  réponses MCP jamais interprétées comme instructions système ; seules les
  données nécessaires à l'appel sont envoyées (pas de contexte projet complet).

## Intégration Chat

L'API `mcp_call` accepte `caller = "chat"` et l'historique distingue
l'appelant. L'affichage des outils et l'appel depuis la conversation, ainsi que
la proposition d'outil par le modèle (tool use des fournisseurs), ne sont pas
encore câblés : cette partie est **non validée** et documentée comme telle.

## Intégration Agent Mode et Team Mode

Les agents d'IH sont des **CLI externes** (claude, codex, gemini, opencode) :
ils ne peuvent pas appeler la couche MCP interne d'IH. L'intégration propre
consiste à leur fournir la configuration MCP (serveur, URL, en-têtes) via
`mcp_export_config` — **sans secrets** (le jeton est à configurer dans le CLI
lui-même). L'injection automatique dans chaque CLI et la restriction des outils
par rôle (Team) restent à faire ; aucune permission n'est accordée par défaut.

## Intégration Automatisations

Le planificateur d'automatisations d'IH (0.6.0) n'appelle pas encore d'outils
MCP : les étapes de workflow sont des prompts d'agents. La règle de sécurité
reste : une exécution sans interaction (pré-autorisée explicitement) ne peut
concerner qu'une **lecture** ; un workflow ne peut jamais contourner
l'approbation, et un workflow désactivé ne déclenche rien.

## Observabilité

- Vue **Historique des appels** : serveur, outil, appelant, statut, durée,
  date, erreur.
- Événements : `ih://mcp-status` (connexion/déconnexion/erreur),
  `ih://mcp-discovered` (outils découverts), `ih://mcp-call`
  (`approval_required`, `started`, `finished`). Jamais de secret.
- Audit : décisions d'approbation (`approved`, `auto`).

## Limites connues

1. **OAuth 2.1** : non intégré (jetons bearer et en-têtes personnalisés
   seulement).
2. **Tool use côté modèle** : les modèles de chat ne proposent pas encore
   d'appel d'outil MCP ; l'invocation est explicite via l'interface.
3. **Injections dans les CLI d'agents** : export manuel de configuration
   uniquement.
4. **Workflows** : pas d'étape « appel d'outil MCP » dans le planificateur.
5. **MCP stdio** : toujours via `ih-extensions` (page Plugins), non migré dans
   `ih-mcp`.
6. **Ressources, prompts, sampling** MCP : non exposés (seuls les outils).
7. **Validation JSON Schema partielle** : objet, propriétés typées simples,
   champs requis, `additionalProperties: false` ; pas de `$ref` distant, pas de
   `oneOf` complet.
8. **Plannificateur de tests GUI** : l'interface a été compilée et les
   commandes câblées ; aucun parcours graphique automatisé n'a été exécuté
   dans cette livraison (voir le rapport de la PR).
