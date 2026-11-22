/**
 * État des onglets de l'éditeur du workspace.
 *
 * Module pur : la lecture disque (`workspace_meta` + `file_read`) et les
 * écritures (`workspace_save`) restent dans le hook React. Ici on ne manipule
 * que des faits déjà chargés, ce qui rend la restauration et la fermeture
 * testables sans WebView.
 */

export interface EditorTab {
  /** Chemin relatif au projet, tel que renvoyé par le noyau. */
  path: string;
  /** Nom de fichier affiché dans l'onglet. */
  title: string;
  /** Contenu du buffer (modifiable). */
  content: string;
  /** Dernier contenu connu sur disque. */
  savedContent: string;
  /** Empreinte du disque au chargement ou après sauvegarde. */
  sha256: string | null;
  /** Taille disque en octets. */
  size: number;
  /** Vrai pour un fichier ouvert en lecture seule (tronqué par exemple). */
  readOnly: boolean;
}

export interface TabState {
  tabs: EditorTab[];
  activePath: string | null;
}

export interface RestoredTabFile {
  path: string;
  content: string;
  sha256: string;
  size: number;
  readOnly?: boolean;
}

/** Nom de fichier d'un chemin projet (séparateur `/` uniquement). */
export function fileTitle(path: string): string {
  const normalized = path.replace(/\\/g, "/");
  const segments = normalized.split("/").filter((segment) => segment.length > 0);
  return segments[segments.length - 1] ?? "";
}

export function createTab(input: {
  path: string;
  content: string;
  sha256: string | null;
  size: number;
  readOnly?: boolean;
}): EditorTab {
  return {
    path: input.path,
    title: fileTitle(input.path),
    content: input.content,
    savedContent: input.content,
    sha256: input.sha256,
    size: input.size,
    readOnly: input.readOnly === true,
  };
}

/** Un onglet est modifié dès que son buffer diffère du disque. */
export function isTabDirty(tab: EditorTab): boolean {
  return tab.content !== tab.savedContent;
}

/** Vrai si l'onglet actif a des modifications non enregistrées. */
export function isActiveTabDirty(tabs: readonly EditorTab[], activePath: string | null): boolean {
  if (!activePath) {
    return false;
  }
  const tab = tabs.find((entry) => entry.path === activePath);
  return tab ? isTabDirty(tab) : false;
}

/**
 * Ouvre un onglet ou active celui qui existe déjà. Un onglet existant n'est
 * jamais dupliqué ni rechargé implicitement : l'utilisateur garde son buffer.
 */
export function openTabState(
  tabs: readonly EditorTab[],
  _activePath: string | null,
  tab: EditorTab,
): TabState {
  const existing = tabs.find((entry) => entry.path === tab.path);
  if (existing) {
    return { tabs: [...tabs], activePath: tab.path };
  }
  return { tabs: [...tabs, tab], activePath: tab.path };
}

/** Change d'onglet actif ; un chemin inconnu laisse l'état inchangé. */
export function activateTabState(
  tabs: readonly EditorTab[],
  activePath: string | null,
  path: string,
): TabState {
  if (!tabs.some((entry) => entry.path === path)) {
    return { tabs: [...tabs], activePath };
  }
  return { tabs: [...tabs], activePath: path };
}

function neighborPath(tabs: readonly EditorTab[], path: string): string | null {
  const index = tabs.findIndex((entry) => entry.path === path);
  if (index < 0) {
    return null;
  }
  const remaining = tabs.filter((entry) => entry.path !== path);
  if (remaining.length === 0) {
    return null;
  }
  return remaining[Math.min(index, remaining.length - 1)]?.path ?? null;
}

/**
 * Ferme un onglet. Si c'était l'onglet actif, l'onglet voisin de droite prend
 * le relais, sinon celui de gauche ; sans onglet restant, aucun actif.
 */
export function closeTabState(
  tabs: readonly EditorTab[],
  activePath: string | null,
  path: string,
): TabState {
  const remaining = tabs.filter((entry) => entry.path !== path);
  if (activePath !== path) {
    return { tabs: remaining, activePath };
  }
  return { tabs: remaining, activePath: neighborPath(tabs, path) };
}

export interface TabClosePlan {
  path: string;
  /** Vrai si l'onglet a des modifications : une confirmation est requise. */
  requiresConfirmation: boolean;
}

/** Décide si la fermeture d'un onglet doit passer par une confirmation. */
export function planCloseTab(tabs: readonly EditorTab[], path: string): TabClosePlan {
  const tab = tabs.find((entry) => entry.path === path);
  return {
    path,
    requiresConfirmation: tab ? isTabDirty(tab) : false,
  };
}

export function updateTabContent(
  tabs: readonly EditorTab[],
  path: string,
  content: string,
): EditorTab[] {
  return tabs.map((tab) => (tab.path === path ? { ...tab, content } : tab));
}

/**
 * Après une écriture vérifiée : `content` est ce qui a réellement été écrit.
 * Le buffer n'est pas remplacé (l'utilisateur a pu continuer à taper pendant
 * l'appel IPC) ; seule la référence disque change, donc un buffer plus récent
 * reste marqué modifié.
 */
export function markTabSaved(
  tabs: readonly EditorTab[],
  path: string,
  input: { content: string; sha256: string; size: number },
): EditorTab[] {
  return tabs.map((tab) =>
    tab.path === path
      ? {
          ...tab,
          savedContent: input.content,
          sha256: input.sha256,
          size: input.size,
          readOnly: false,
        }
      : tab,
  );
}

/** Après un rechargement disque : remplace le buffer et l'empreinte. */
export function reloadTab(
  tabs: readonly EditorTab[],
  path: string,
  input: { content: string; sha256: string; size: number; readOnly?: boolean },
): EditorTab[] {
  return tabs.map((tab) =>
    tab.path === path
      ? {
          ...tab,
          content: input.content,
          savedContent: input.content,
          sha256: input.sha256,
          size: input.size,
          readOnly: input.readOnly === true || tab.readOnly,
        }
      : tab,
  );
}

/**
 * Reconstruit les onglets depuis les chemins persistés et les fichiers
 * effectivement relus. Un fichier disparu (ou illisible) est ignoré
 * silencieusement, sans interrompre la restauration des autres.
 */
export function restoreTabs(
  persistedPaths: readonly string[],
  loaded: readonly RestoredTabFile[],
  activePath: string | null = null,
): TabState {
  const byPath = new Map(loaded.map((entry) => [entry.path, entry]));
  const restored: EditorTab[] = [];
  for (const path of persistedPaths) {
    const file = byPath.get(path);
    if (!file) {
      continue;
    }
    restored.push(
      createTab({
        path: file.path,
        content: file.content,
        sha256: file.sha256,
        size: file.size,
        readOnly: file.readOnly,
      }),
    );
  }
  const active =
    activePath !== null && restored.some((tab) => tab.path === activePath)
      ? activePath
      : restored[0]?.path ?? null;
  return { tabs: restored, activePath: active };
}
