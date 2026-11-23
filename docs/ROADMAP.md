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
| **Release engineering partiel** — signature Ed25519, `.deb`, AppImage, PKGBUILD | `ih-update` (39 tests) + bundles produits + CI Arch |

## Restant, par ordre de valeur

1. **Écran de chat fournisseur** (streaming déjà disponible côté noyau) et
   persistance multi-tours dans un thread.
2. **Planificateur d'automatisations en arrière-plan** (aujourd'hui : exécution
   manuelle) + édition/suppression des workflows.
3. **Registre de skills distant** avec vérification de mise à jour.
4. **MCP étendu** : ressources, prompts, transport HTTP/SSE.
5. **Git distant** : `fetch`, `pull`, `push`, résolution de conflits.
6. **Coloration syntaxique** dans l'éditeur.
7. **Page d'audit** : exposer `list_audit` via une commande IPC et enregistrer
   les décisions d'approbation.
8. **Boucle autonome d'équipe** : relance automatique des tâches prêtes sans
   intervention, avec budget et limite d'itérations.
9. **Validation sur Arch physique** en plus du conteneur CI.
10. ~~Choix de licence et fichier `LICENSE`~~ → **MIT** (fait).

## Règle d'acceptation

Un jalon n'est coché que si : le code est connecté à l'interface, les tests
automatisés passent, et le comportement a été observé au moins une fois sur une
machine réelle. Tout le reste est listé comme partiel dans
[docs/FEATURES.md](FEATURES.md).
