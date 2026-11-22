//! Espace de travail IH : Git, édition sûre des fichiers de projet et terminal
//! PTY natif.
//!
//! Ce crate ne simule rien : Git est piloté en spawnant le binaire `git`
//! (arguments séparés, jamais de shell) et le terminal s'appuie sur un vrai
//! pseudo-terminal via `portable-pty`.
//!
//! ## Limites de sûreté
//!
//! - [`ProjectFs`] est une prison de chemins : toute résolution est
//!   canonicalisée puis vérifiée comme descendant de la racine du projet.
//! - Les commandes Git refusent les chemins contenant `..`, absolus ou
//!   commençant par `-` (injection d'arguments).
//! - Les fichiers binaires (octet nul) et les fichiers de plus de 2 Mio sont
//!   ignorés par la recherche et bornés en lecture.

pub mod diff;
pub mod fs;
pub mod git;
pub mod pty;

pub use diff::{diff_lines, DiffKind, DiffLine, Hunk, LineDiff};
pub use fs::{FileContent, FileMeta, FileNode, ProjectFs, SearchHit, WriteOutcome};
pub use git::{CommitInfo, GitRepo, GitStatus, StatusEntry, Worktree};
pub use pty::{read_chunk, AsyncPtyReader, PtySession};

/// Taille maximale d'un fichier texte lu ou indexé par [`ProjectFs`].
pub const MAX_TEXT_FILE_BYTES: usize = 2 * 1024 * 1024;

/// Taille maximale d'un diff Git renvoyé à l'interface.
pub const MAX_DIFF_BYTES: usize = 2 * 1024 * 1024;
