# Feuille de route d'IH

Cette feuille de route remplace les « portes de livraison » théoriques de la
fondation d'origine : elle décrit ce qui est franchi et ce qui reste, sans
prétendre qu'un jalon est atteint s'il n'est pas testé.

## Franchis

| Jalon | Preuve |
|---|---|
| **Fondation** — shell Tauri, i18n FR/EN, CI, capacités minimales | build release + bundles ; 800 clés par langue ; CI verte |
| **Espace de travail** — projets, fichiers, éditeur, historique | `ih-workspace` (21 tests) + `ih-store` (15 tests) + pages dédiées |
| **Runtime d'agents** — supervision, streaming, annulation, limite | `ih-agents` (13 tests), dont timeout, annulation et sortie bornée |
| **Fournisseurs IA** — cloud + local, modèles, santé, streaming | `ih-providers` (15 tests) sur 4 fournisseurs, sans réseau externe |
| **Skills et plugins** — installation vérifiée, client MCP | `ih-extensions` (27 tests) |
| **Équipes et automatisation** — graphe, rôles, ordonnanceur | `ih-orchestrator` (23 tests) |
| **Automatisations actives** — planificateur d'arrière-plan, édition/suppression des workflows, événements temps réel | `ih-store` (workflows dus/suppression) + `src-tauri/src/automation.rs` + `ih://workflow-run` |
| **Release engineering partiel** — signature Ed25519, `.deb`, AppImage, PKGBUILD | `ih-update` (39 tests) + bundles produits + CI Arch |

## Restant, par ordre de valeur

1. **Registre de skills distant** avec vérification de mise à jour.
2. **MCP étendu** : ressources, prompts, transport HTTP/SSE.
3. **Git distant** : `fetch`, `pull`, `push`, résolution de conflits.
4. **Page d'audit** : exposer `list_audit` via une commande IPC et enregistrer
   les décisions d'approbation.
5. **Boucle autonome d'équipe** : relance automatique des tâches prêtes sans
   intervention, avec budget et limite d'itérations.
6. **Validation sur Arch physique** en plus du conteneur CI.

## Règle d'acceptation

Un jalon n'est coché que si : le code est connecté à l'interface, les tests
automatisés passent, et le comportement a été observé au moins une fois sur une
machine réelle. Tout le reste est listé comme partiel dans
[docs/FEATURES.md](FEATURES.md).
