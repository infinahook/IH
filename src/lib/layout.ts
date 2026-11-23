/**
 * Disposition du workspace : largeurs des zones, panneau bas et onglets.
 *
 * Les fonctions de ce module sont pures et testables : le composant ne fait
 * que transporter l'état, le clamp et la (dé)sérialisation restent ici.
 * La charge sauvegardée est un JSON opaque pour le noyau (`layout_save`).
 */

export const LAYOUT_VERSION = 1;

/**
 * Version de la disposition « UX ». Elle ne touche pas au schéma lu par le
 * noyau (toujours `LAYOUT_VERSION`) : elle sert uniquement à migrer une fois
 * les largeurs historiques vers la nouvelle répartition conversation-first.
 */
export const LAYOUT_UX_VERSION = 2;

export type BottomTab = "terminal" | "git" | "problems";

export const BOTTOM_TABS: readonly BottomTab[] = ["terminal", "git", "problems"];

export interface WorkspaceLayout {
  version: number;
  /** Version des valeurs par défaut (voir `migrateLayout`). */
  ux_version: number;
  /** Projet auquel appartiennent les onglets persistés. */
  project_id: string | null;
  conversation_width: number;
  explorer_width: number;
  explorer_visible: boolean;
  bottom_height: number;
  bottom_open: boolean;
  bottom_tab: BottomTab;
  tabs: string[];
  active_tab: string | null;
}

export interface LayoutBounds {
  /** Largeur utile de la zone de travail, en pixels. */
  width: number;
  /** Hauteur utile de la zone de travail, en pixels. */
  height: number;
}

/**
 * La conversation est le point focal : elle occupe au moins 55 % de la zone
 * utile par défaut. L'éditeur conserve toujours une largeur exploitable.
 */
export const MIN_CONVERSATION_WIDTH = 420;
export const MIN_EDITOR_WIDTH = 320;
/** Alias explicite : le workspace (explorateur + éditeur) ne descend pas sous 320 px. */
export const MIN_WORKSPACE_WIDTH = MIN_EDITOR_WIDTH;
export const MAX_CONVERSATION_RATIO = 0.78;
export const DEFAULT_CONVERSATION_RATIO = 0.66;

export const MIN_EXPLORER_WIDTH = 180;
export const MAX_EXPLORER_WIDTH = 520;

export const MIN_BOTTOM_HEIGHT = 150;
export const MAX_BOTTOM_RATIO = 0.6;
export const MIN_WORKSPACE_HEIGHT = 300;

/** En dessous de cette largeur de fenêtre, le workspace passe en mode replié. */
export const COMPACT_BREAKPOINT = 1200;

export const DEFAULT_LAYOUT: WorkspaceLayout = {
  version: LAYOUT_VERSION,
  ux_version: LAYOUT_UX_VERSION,
  project_id: null,
  conversation_width: 760,
  explorer_width: 270,
  explorer_visible: true,
  bottom_height: 240,
  bottom_open: false,
  bottom_tab: "terminal",
  tabs: [],
  active_tab: null,
};

const DEFAULT_BOUNDS: LayoutBounds = { width: 1440, height: 900 };

function finiteNumber(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(Math.max(value, min), max);
}

function safeBounds(bounds: LayoutBounds | null | undefined): LayoutBounds {
  const width = finiteNumber(bounds?.width);
  const height = finiteNumber(bounds?.height);
  return {
    width: width !== null && width > 0 ? width : DEFAULT_BOUNDS.width,
    height: height !== null && height > 0 ? height : DEFAULT_BOUNDS.height,
  };
}

/** Largeur maximale de la conversation : l'éditeur garde toujours sa place. */
export function maxConversationWidth(bounds: LayoutBounds): number {
  const safe = safeBounds(bounds);
  return Math.max(
    MIN_CONVERSATION_WIDTH,
    Math.min(safe.width * MAX_CONVERSATION_RATIO, safe.width - MIN_EDITOR_WIDTH),
  );
}

/**
 * Largeur de conversation par défaut : environ 66 % de la zone utile, sans
 * jamais descendre sous 420 px ni affamer l'éditeur. La valeur est calculée à
 * partir des dimensions réelles pour rester ≥ 55 % à toutes les tailles.
 */
export function defaultConversationWidth(bounds: LayoutBounds): number {
  const safe = safeBounds(bounds);
  const preferred = Math.round(safe.width * DEFAULT_CONVERSATION_RATIO);
  return clamp(preferred, MIN_CONVERSATION_WIDTH, maxConversationWidth(safe));
}

/** Vrai quand la fenêtre est trop étroite pour garder le workspace côte à côte. */
export function isCompactWidth(width: number): boolean {
  const safe = finiteNumber(width);
  return safe === null || safe < COMPACT_BREAKPOINT;
}

/**
 * Disposition d'ouverture pour un projet sans disposition sauvegardée :
 * conversation majoritaire et explorateur replié. Les onglets restent gérés
 * par `useWorkspace`.
 */
export function defaultWorkspaceLayout(bounds: LayoutBounds): WorkspaceLayout {
  return {
    ...DEFAULT_LAYOUT,
    ux_version: LAYOUT_UX_VERSION,
    conversation_width: defaultConversationWidth(bounds),
    explorer_visible: false,
  };
}

/**
 * Migration unique d'une disposition antérieure : on applique la nouvelle
 * répartition conversation-first sans toucher aux onglets, au panneau bas ni
 * au projet. Une disposition déjà migrée est renvoyée telle quelle.
 */
export function migrateLayout(
  layout: WorkspaceLayout,
  bounds: LayoutBounds,
): WorkspaceLayout {
  if (layout.ux_version >= LAYOUT_UX_VERSION) {
    return layout;
  }
  return {
    ...layout,
    ux_version: LAYOUT_UX_VERSION,
    conversation_width: defaultConversationWidth(bounds),
    explorer_visible: false,
  };
}

export function clampConversationWidth(value: number, bounds: LayoutBounds): number {
  const number = finiteNumber(value);
  const fallback = DEFAULT_LAYOUT.conversation_width;
  return clamp(
    number === null ? fallback : number,
    MIN_CONVERSATION_WIDTH,
    maxConversationWidth(bounds),
  );
}

export function maxExplorerWidth(bounds: LayoutBounds): number {
  const safe = safeBounds(bounds);
  return Math.max(
    MIN_EXPLORER_WIDTH,
    Math.min(MAX_EXPLORER_WIDTH, safe.width - MIN_CONVERSATION_WIDTH - MIN_EDITOR_WIDTH),
  );
}

export function clampExplorerWidth(value: number, bounds: LayoutBounds): number {
  const number = finiteNumber(value);
  const fallback = DEFAULT_LAYOUT.explorer_width;
  return clamp(number === null ? fallback : number, MIN_EXPLORER_WIDTH, maxExplorerWidth(bounds));
}

export function maxBottomHeight(bounds: LayoutBounds): number {
  const safe = safeBounds(bounds);
  return Math.max(
    MIN_BOTTOM_HEIGHT,
    Math.min(safe.height * MAX_BOTTOM_RATIO, safe.height - MIN_WORKSPACE_HEIGHT),
  );
}

export function clampBottomHeight(value: number, bounds: LayoutBounds): number {
  const number = finiteNumber(value);
  const fallback = DEFAULT_LAYOUT.bottom_height;
  return clamp(number === null ? fallback : number, MIN_BOTTOM_HEIGHT, maxBottomHeight(bounds));
}

export function normalizeBottomTab(value: unknown): BottomTab {
  return typeof value === "string" && (BOTTOM_TABS as readonly string[]).includes(value)
    ? (value as BottomTab)
    : DEFAULT_LAYOUT.bottom_tab;
}

/** Déduplique et nettoie les chemins d'onglets (jamais de chaîne vide). */
export function normalizeTabPaths(value: unknown): string[] {
  if (!Array.isArray(value)) {
    return [];
  }
  const seen = new Set<string>();
  const paths: string[] = [];
  for (const entry of value) {
    if (typeof entry !== "string") {
      continue;
    }
    const path = entry.trim();
    if (path.length === 0 || seen.has(path)) {
      continue;
    }
    seen.add(path);
    paths.push(path);
  }
  return paths;
}

/**
 * Ramène une disposition arbitraire dans les bornes de la zone de travail.
 * Les champs absents ou invalides reprennent leur valeur par défaut.
 */
export function clampLayout(layout: WorkspaceLayout, bounds: LayoutBounds): WorkspaceLayout {
  const safe = safeBounds(bounds);
  const tabs = normalizeTabPaths(layout.tabs);
  const active =
    typeof layout.active_tab === "string" && tabs.includes(layout.active_tab)
      ? layout.active_tab
      : null;
  const explorerVisible = layout.explorer_visible !== false;
  let conversationWidth = clampConversationWidth(layout.conversation_width, bounds);
  let explorerWidth = clampExplorerWidth(layout.explorer_width, bounds);
  // La conversation et l'explorateur ne doivent jamais affamer l'éditeur :
  // on réduit d'abord l'explorateur, puis la conversation si nécessaire.
  const limit = safe.width - MIN_EDITOR_WIDTH - (explorerVisible ? 14 : 7);
  if (explorerVisible && conversationWidth + explorerWidth > limit) {
    let overflow = conversationWidth + explorerWidth - limit;
    const shrinkExplorer = Math.min(
      overflow,
      Math.max(0, explorerWidth - MIN_EXPLORER_WIDTH),
    );
    explorerWidth -= shrinkExplorer;
    overflow -= shrinkExplorer;
    if (overflow > 0) {
      conversationWidth = Math.max(MIN_CONVERSATION_WIDTH, conversationWidth - overflow);
    }
  }
  return {
    version: LAYOUT_VERSION,
    ux_version:
      typeof layout.ux_version === "number" && Number.isFinite(layout.ux_version)
        ? layout.ux_version
        : LAYOUT_UX_VERSION,
    project_id: typeof layout.project_id === "string" ? layout.project_id : null,
    conversation_width: conversationWidth,
    explorer_width: explorerWidth,
    explorer_visible: explorerVisible,
    bottom_height: clampBottomHeight(layout.bottom_height, bounds),
    bottom_open: layout.bottom_open === true,
    bottom_tab: normalizeBottomTab(layout.bottom_tab),
    tabs,
    active_tab: active,
  };
}

function readNumber(
  record: Record<string, unknown>,
  snake: string,
  camel: string,
  fallback: number,
  min: number,
  max: number,
): number {
  const raw = finiteNumber(record[snake]) ?? finiteNumber(record[camel]);
  if (raw === null || raw < min || raw > max) {
    return fallback;
  }
  return raw;
}

/**
 * Relit la disposition sauvegardée. Toute charge illisible, version inconnue
 * ou valeur aberrante retombe sur la disposition par défaut : le workspace ne
 * doit jamais refuser de s'ouvrir à cause d'un réglage corrompu.
 */
export function parseLayout(raw: string | null | undefined): WorkspaceLayout {
  if (typeof raw !== "string" || raw.trim().length === 0) {
    return { ...DEFAULT_LAYOUT };
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return { ...DEFAULT_LAYOUT };
  }
  if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
    return { ...DEFAULT_LAYOUT };
  }
  const record = parsed as Record<string, unknown>;
  if (record.version !== undefined && record.version !== LAYOUT_VERSION) {
    return { ...DEFAULT_LAYOUT };
  }
  const tabs = normalizeTabPaths(record.tabs);
  const activeRaw = record.active_tab ?? record.activeTab;
  return {
    version: LAYOUT_VERSION,
    // Une charge historique n'a pas ce champ : 0 déclenche la migration UX.
    ux_version: readNumber(record, "ux_version", "uxVersion", 0, 0, 1000),
    project_id:
      typeof record.project_id === "string"
        ? record.project_id
        : typeof record.projectId === "string"
          ? record.projectId
          : null,
    conversation_width: readNumber(
      record,
      "conversation_width",
      "conversationWidth",
      DEFAULT_LAYOUT.conversation_width,
      MIN_CONVERSATION_WIDTH,
      4000,
    ),
    explorer_width: readNumber(
      record,
      "explorer_width",
      "explorerWidth",
      DEFAULT_LAYOUT.explorer_width,
      MIN_EXPLORER_WIDTH,
      MAX_EXPLORER_WIDTH,
    ),
    explorer_visible: record.explorer_visible !== false && record.explorerVisible !== false,
    bottom_height: readNumber(
      record,
      "bottom_height",
      "bottomHeight",
      DEFAULT_LAYOUT.bottom_height,
      MIN_BOTTOM_HEIGHT,
      2000,
    ),
    bottom_open: record.bottom_open === true || record.bottomOpen === true,
    bottom_tab: normalizeBottomTab(record.bottom_tab ?? record.bottomTab),
    tabs,
    active_tab: typeof activeRaw === "string" && tabs.includes(activeRaw) ? activeRaw : null,
  };
}

/** Sérialise la disposition avec un ordre de clés stable (JSON opaque côté Rust). */
export function serializeLayout(layout: WorkspaceLayout): string {
  const payload: WorkspaceLayout = {
    version: LAYOUT_VERSION,
    ux_version:
      typeof layout.ux_version === "number" && Number.isFinite(layout.ux_version)
        ? layout.ux_version
        : LAYOUT_UX_VERSION,
    project_id: layout.project_id,
    conversation_width: Math.round(layout.conversation_width),
    explorer_width: Math.round(layout.explorer_width),
    explorer_visible: layout.explorer_visible,
    bottom_height: Math.round(layout.bottom_height),
    bottom_open: layout.bottom_open,
    bottom_tab: normalizeBottomTab(layout.bottom_tab),
    tabs: normalizeTabPaths(layout.tabs),
    active_tab: layout.active_tab,
  };
  return JSON.stringify(payload);
}

/** Vrai si la disposition ne diffère pas de la disposition par défaut. */
export function isDefaultLayout(layout: WorkspaceLayout): boolean {
  return (
    layout.conversation_width === DEFAULT_LAYOUT.conversation_width &&
    layout.explorer_width === DEFAULT_LAYOUT.explorer_width &&
    layout.explorer_visible === DEFAULT_LAYOUT.explorer_visible &&
    layout.bottom_height === DEFAULT_LAYOUT.bottom_height &&
    layout.bottom_open === DEFAULT_LAYOUT.bottom_open &&
    layout.bottom_tab === DEFAULT_LAYOUT.bottom_tab
  );
}
