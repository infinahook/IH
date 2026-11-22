//! Calcul de différences ligne à ligne, sans dépendance externe.
//!
//! Utilisé pour préparer un diff avant/après avant toute écriture : IH ne
//! remplace jamais un fichier sans que l'utilisateur ait vu les lignes
//! ajoutées et supprimées.
//!
//! Le calcul est borné : au-delà de [`MAX_DIFF_LINES`] lignes dans la partie
//! modifiée, le diff bascule en remplacement global marqué `truncated`, pour ne
//! jamais bloquer l'interface ni saturer la mémoire.

use serde::{Deserialize, Serialize};

/// Nombre maximal de lignes traitées par le calcul LCS.
pub const MAX_DIFF_LINES: usize = 2_000;
/// Lignes de contexte autour de chaque changement.
pub const CONTEXT_LINES: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiffKind {
    Context,
    Add,
    Remove,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffLine {
    pub kind: DiffKind,
    pub text: String,
    /// Numéro de ligne dans l'ancien fichier (1-based), absent pour un ajout.
    pub old_line: Option<usize>,
    /// Numéro de ligne dans le nouveau fichier (1-based), absent pour une suppression.
    pub new_line: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hunk {
    pub old_start: usize,
    pub old_len: usize,
    pub new_start: usize,
    pub new_len: usize,
    pub lines: Vec<DiffLine>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineDiff {
    pub hunks: Vec<Hunk>,
    pub additions: usize,
    pub deletions: usize,
    /// Vrai si le diff a été simplifié faute de budget de calcul.
    pub truncated: bool,
    /// Nombre total de lignes de l'ancien et du nouveau contenu.
    pub old_lines: usize,
    pub new_lines: usize,
}

impl LineDiff {
    pub fn is_empty(&self) -> bool {
        self.additions == 0 && self.deletions == 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Keep(usize, usize),
    Remove(usize),
    Add(usize),
}

/// Calcule la différence entre deux contenus texte.
pub fn diff_lines(old: &str, new: &str) -> LineDiff {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();
    let (old_len, new_len) = (old_lines.len(), new_lines.len());

    if old == new {
        return LineDiff {
            hunks: Vec::new(),
            additions: 0,
            deletions: 0,
            truncated: false,
            old_lines: old_len,
            new_lines: new_len,
        };
    }

    // Préfixe commun.
    let mut prefix = 0;
    while prefix < old_len && prefix < new_len && old_lines[prefix] == new_lines[prefix] {
        prefix += 1;
    }
    // Suffixe commun (sans chevaucher le préfixe).
    let mut suffix = 0;
    while suffix < (old_len - prefix)
        && suffix < (new_len - prefix)
        && old_lines[old_len - 1 - suffix] == new_lines[new_len - 1 - suffix]
    {
        suffix += 1;
    }

    let old_mid = &old_lines[prefix..old_len - suffix];
    let new_mid = &new_lines[prefix..new_len - suffix];

    let truncated = old_mid.len() > MAX_DIFF_LINES || new_mid.len() > MAX_DIFF_LINES;
    let ops = if truncated {
        // Repli : tout le milieu est remplacé.
        let mut ops = Vec::with_capacity(old_mid.len() + new_mid.len());
        for (index, _) in old_mid.iter().enumerate() {
            ops.push(Op::Remove(index));
        }
        for (index, _) in new_mid.iter().enumerate() {
            ops.push(Op::Add(index));
        }
        ops
    } else {
        lcs_ops(old_mid, new_mid)
    };

    let mut lines: Vec<DiffLine> = Vec::new();
    for (index, line) in old_lines[..prefix].iter().enumerate() {
        lines.push(DiffLine {
            kind: DiffKind::Context,
            text: (*line).to_string(),
            old_line: Some(index + 1),
            new_line: Some(index + 1),
        });
    }
    for op in &ops {
        match *op {
            Op::Keep(i, j) => lines.push(DiffLine {
                kind: DiffKind::Context,
                text: old_mid[i].to_string(),
                old_line: Some(prefix + i + 1),
                new_line: Some(prefix + j + 1),
            }),
            Op::Remove(i) => lines.push(DiffLine {
                kind: DiffKind::Remove,
                text: old_mid[i].to_string(),
                old_line: Some(prefix + i + 1),
                new_line: None,
            }),
            Op::Add(j) => lines.push(DiffLine {
                kind: DiffKind::Add,
                text: new_mid[j].to_string(),
                old_line: None,
                new_line: Some(prefix + j + 1),
            }),
        }
    }
    for index in 0..suffix {
        let old_index = old_len - suffix + index;
        let new_index = new_len - suffix + index;
        lines.push(DiffLine {
            kind: DiffKind::Context,
            text: old_lines[old_index].to_string(),
            old_line: Some(old_index + 1),
            new_line: Some(new_index + 1),
        });
    }

    let additions = lines.iter().filter(|l| l.kind == DiffKind::Add).count();
    let deletions = lines.iter().filter(|l| l.kind == DiffKind::Remove).count();

    LineDiff {
        hunks: group_hunks(&lines),
        additions,
        deletions,
        truncated,
        old_lines: old_len,
        new_lines: new_len,
    }
}

/// LCS classique sur la partie réellement modifiée (bornée par l'appelant).
fn lcs_ops(a: &[&str], b: &[&str]) -> Vec<Op> {
    let (n, m) = (a.len(), b.len());
    let mut table = vec![0u32; (n + 1) * (m + 1)];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            table[i * (m + 1) + j] = if a[i] == b[j] {
                table[(i + 1) * (m + 1) + j + 1] + 1
            } else {
                table[(i + 1) * (m + 1) + j].max(table[i * (m + 1) + j + 1])
            };
        }
    }

    let mut ops = Vec::with_capacity(n + m);
    let (mut i, mut j) = (0usize, 0usize);
    while i < n && j < m {
        if a[i] == b[j] {
            ops.push(Op::Keep(i, j));
            i += 1;
            j += 1;
        } else if table[(i + 1) * (m + 1) + j] >= table[i * (m + 1) + j + 1] {
            ops.push(Op::Remove(i));
            i += 1;
        } else {
            ops.push(Op::Add(j));
            j += 1;
        }
    }
    while i < n {
        ops.push(Op::Remove(i));
        i += 1;
    }
    while j < m {
        ops.push(Op::Add(j));
        j += 1;
    }
    ops
}

/// Regroupe les lignes en hunks avec `CONTEXT_LINES` de contexte.
fn group_hunks(lines: &[DiffLine]) -> Vec<Hunk> {
    let changed: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| line.kind != DiffKind::Context)
        .map(|(index, _)| index)
        .collect();
    if changed.is_empty() {
        return Vec::new();
    }

    let mut ranges: Vec<(usize, usize)> = Vec::new();
    for &index in &changed {
        let start = index.saturating_sub(CONTEXT_LINES);
        let end = (index + CONTEXT_LINES + 1).min(lines.len());
        match ranges.last_mut() {
            Some((_, last_end)) if start <= *last_end => *last_end = (*last_end).max(end),
            _ => ranges.push((start, end)),
        }
    }

    ranges
        .into_iter()
        .map(|(start, end)| {
            let slice = &lines[start..end];
            let old_start = slice.iter().find_map(|line| line.old_line).unwrap_or(0);
            let new_start = slice.iter().find_map(|line| line.new_line).unwrap_or(0);
            let old_len = slice.iter().filter(|line| line.old_line.is_some()).count();
            let new_len = slice.iter().filter(|line| line.new_line.is_some()).count();
            Hunk {
                old_start,
                old_len,
                new_start,
                new_len,
                lines: slice.to_vec(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(diff: &LineDiff) -> Vec<DiffKind> {
        diff.hunks
            .iter()
            .flat_map(|h| h.lines.iter().map(|l| l.kind))
            .collect()
    }

    #[test]
    fn contenus_identiques_donnent_un_diff_vide() {
        let diff = diff_lines("a\nb\n", "a\nb\n");
        assert!(diff.is_empty());
        assert!(diff.hunks.is_empty());
        assert_eq!(diff.old_lines, 2);
    }

    #[test]
    fn ajout_de_ligne() {
        let diff = diff_lines("a\nb\n", "a\nx\nb\n");
        assert_eq!(diff.additions, 1);
        assert_eq!(diff.deletions, 0);
        let added: Vec<&str> = diff
            .hunks
            .iter()
            .flat_map(|h| h.lines.iter())
            .filter(|l| l.kind == DiffKind::Add)
            .map(|l| l.text.as_str())
            .collect();
        assert_eq!(added, vec!["x"]);
    }

    #[test]
    fn suppression_de_ligne() {
        let diff = diff_lines("a\nb\nc\n", "a\nc\n");
        assert_eq!(diff.deletions, 1);
        assert_eq!(diff.additions, 0);
    }

    #[test]
    fn modification_de_ligne() {
        let diff = diff_lines("let a = 1;\n", "let a = 2;\n");
        assert_eq!(diff.additions, 1);
        assert_eq!(diff.deletions, 1);
        let all: Vec<&DiffLine> = diff.hunks.iter().flat_map(|h| h.lines.iter()).collect();
        assert!(all
            .iter()
            .any(|l| l.kind == DiffKind::Remove && l.text.contains('1')));
        assert!(all
            .iter()
            .any(|l| l.kind == DiffKind::Add && l.text.contains('2')));
    }

    #[test]
    fn numeros_de_lignes_coherents() {
        let diff = diff_lines("a\nb\nc\nd\ne\n", "a\nb\nX\nd\ne\n");
        for hunk in &diff.hunks {
            for line in &hunk.lines {
                match line.kind {
                    DiffKind::Remove => assert!(line.old_line.is_some() && line.new_line.is_none()),
                    DiffKind::Add => assert!(line.old_line.is_none() && line.new_line.is_some()),
                    DiffKind::Context => {
                        assert_eq!(line.old_line, line.new_line);
                    }
                }
            }
        }
    }

    #[test]
    fn gros_fichier_bascule_en_repli_borne() {
        let old: String = (0..(MAX_DIFF_LINES * 2))
            .map(|i| format!("l{i}\n"))
            .collect();
        let new: String = (0..(MAX_DIFF_LINES * 2))
            .map(|i| format!("n{i}\n"))
            .collect();
        let diff = diff_lines(&old, &new);
        assert!(diff.truncated);
        assert!(diff.additions > 0 && diff.deletions > 0);
    }

    #[test]
    fn fichiers_vides() {
        assert!(diff_lines("", "").is_empty());
        let diff = diff_lines("", "a\n");
        assert_eq!(diff.additions, 1);
        let diff = diff_lines("a\n", "");
        assert_eq!(diff.deletions, 1);
    }

    #[test]
    fn hunks_separes_pour_changements_eloignes() {
        let old: String = (0..40).map(|i| format!("l{i}\n")).collect();
        let new: String = (0..40)
            .map(|i| {
                if i == 2 || i == 30 {
                    format!("X{i}\n")
                } else {
                    format!("l{i}\n")
                }
            })
            .collect();
        let diff = diff_lines(&old, &new);
        assert_eq!(
            diff.hunks.len(),
            2,
            "changements éloignés = hunks distincts"
        );
        assert_eq!(
            kinds(&diff).iter().filter(|k| **k == DiffKind::Add).count(),
            2
        );
    }
}
