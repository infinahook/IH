/**
 * État pur de la refonte UX de la conversation.
 *
 * Ce module ne dépend ni de React ni du DOM : il centralise les décisions
 * testables (disponibilité fournisseur/modèle, drawer, conservation de la
 * conversation au changement de fournisseur) pour que les composants ne
 * fassent que transporter l'état.
 */

import type { Message, ProviderKind, ProviderView } from "./api";
import { providerRequiresSecret } from "./providers";

// ------------------------------------------------------- disponibilité

export type ProviderAvailabilityTone = "success" | "warning" | "neutral" | "danger";

export interface ProviderAvailability {
  tone: ProviderAvailabilityTone;
  /** Clé i18n du libellé court affiché dans la pastille. */
  labelKey: string;
  /** Clé i18n de l'infobulle explicative. */
  hintKey: string;
  ready: boolean;
  requiresSecret: boolean;
}

/** Sous-ensemble de `ProviderView` utile au calcul (facilite les tests). */
export type ProviderAvailabilitySource = Pick<
  ProviderView,
  "enabled" | "kind" | "has_secret" | "secret_stored" | "default_model" | "models"
>;

export interface ProviderAvailabilityInput {
  provider: ProviderAvailabilitySource | null;
  model: string;
  /** `providers_list` n'a pas encore répondu. */
  loading?: boolean;
  /** Une erreur de chargement/rafraîchissement des fournisseurs est active. */
  error?: boolean;
}

function hasStoredSecret(provider: ProviderAvailabilitySource): boolean {
  return provider.has_secret || provider.secret_stored;
}

/** Un fournisseur local (Ollama) n'exige aucune clé ; les autres, si. */
export function providerRequiresKey(kind: ProviderKind): boolean {
  return providerRequiresSecret(kind);
}

/**
 * Calcule la pastille de la barre de conversation :
 * vert = actif + modèle + clé si requise ; orange = clé ou modèle manquant ;
 * gris = désactivé/aucun fournisseur ; rouge = erreur de fournisseur.
 */
export function providerAvailability(
  input: ProviderAvailabilityInput,
): ProviderAvailability {
  const { provider, model, loading = false, error = false } = input;
  if (loading) {
    return {
      tone: "neutral",
      labelKey: "chat.status_loading",
      hintKey: "chat.status_loading_hint",
      ready: false,
      requiresSecret: false,
    };
  }
  if (error) {
    return {
      tone: "danger",
      labelKey: "chat.status_error",
      hintKey: "chat.status_error_hint",
      ready: false,
      requiresSecret: false,
    };
  }
  if (!provider) {
    return {
      tone: "neutral",
      labelKey: "chat.status_no_provider",
      hintKey: "chat.status_no_provider_hint",
      ready: false,
      requiresSecret: false,
    };
  }
  const requiresSecret = providerRequiresKey(provider.kind);
  if (!provider.enabled) {
    return {
      tone: "neutral",
      labelKey: "chat.status_disabled",
      hintKey: "chat.status_disabled_hint",
      ready: false,
      requiresSecret,
    };
  }
  if (requiresSecret && !hasStoredSecret(provider)) {
    return {
      tone: "warning",
      labelKey: "chat.status_missing_key",
      hintKey: "chat.status_missing_key_hint",
      ready: false,
      requiresSecret,
    };
  }
  if (model.trim().length === 0) {
    return {
      tone: "warning",
      labelKey: "chat.status_no_model",
      hintKey: "chat.status_no_model_hint",
      ready: false,
      requiresSecret,
    };
  }
  return {
    tone: "success",
    labelKey: "chat.status_ready",
    hintKey: "chat.status_ready_hint",
    ready: true,
    requiresSecret,
  };
}

// ------------------------------------- conservation au changement de fournisseur

export interface ConversationSelectionState {
  providerId: string;
  model: string;
  threadId: string | null;
  messages: readonly Message[];
}

export interface ProviderSelectionSource {
  id: string;
  default_model: string | null;
  models: ReadonlyArray<{ id: string }>;
}

/**
 * Applique un changement de fournisseur sans jamais toucher au fil ni aux
 * messages : seul le couple `{ providerId, model }` évolue. C'est le contrat
 * vérifié par les tests et utilisé par la barre de conversation.
 */
export function changeProviderSelection(
  state: ConversationSelectionState,
  provider: ProviderSelectionSource | null,
  fallbackModel = "",
): ConversationSelectionState {
  const model = provider
    ? provider.default_model?.trim() || provider.models[0]?.id || fallbackModel
    : "";
  return {
    ...state,
    providerId: provider?.id ?? "",
    model,
  };
}

// --------------------------------------------------------------- drawer

export type ProviderDrawerSection =
  | "providers"
  | "models"
  | "endpoints"
  | "keys"
  | "advanced";

export const PROVIDER_DRAWER_SECTIONS: readonly ProviderDrawerSection[] = [
  "providers",
  "models",
  "endpoints",
  "keys",
  "advanced",
];

export interface ProviderDrawerState {
  open: boolean;
  /** Dernière section active (celle qui a le focus sémantique). */
  section: ProviderDrawerSection;
  /** Sections dépliées, dans l'ordre d'ouverture. */
  expanded: ProviderDrawerSection[];
}

export const INITIAL_PROVIDER_DRAWER_STATE: ProviderDrawerState = {
  open: false,
  section: "providers",
  expanded: ["providers", "models"],
};

export type ProviderDrawerAction =
  | { type: "open" }
  | { type: "close" }
  | { type: "select_section"; section: ProviderDrawerSection }
  | { type: "toggle_section"; section: ProviderDrawerSection };

function expand(
  expanded: readonly ProviderDrawerSection[],
  section: ProviderDrawerSection,
): ProviderDrawerSection[] {
  return expanded.includes(section) ? [...expanded] : [...expanded, section];
}

/** Réducteur pur du panneau de configuration secondaire. */
export function providerDrawerReducer(
  state: ProviderDrawerState,
  action: ProviderDrawerAction,
): ProviderDrawerState {
  switch (action.type) {
    case "open":
      return { ...state, open: true, expanded: expand(state.expanded, state.section) };
    case "close":
      return { ...state, open: false };
    case "select_section":
      return {
        ...state,
        open: true,
        section: action.section,
        expanded: expand(state.expanded, action.section),
      };
    case "toggle_section": {
      const isExpanded = state.expanded.includes(action.section);
      return {
        ...state,
        open: true,
        section: action.section,
        expanded: isExpanded
          ? state.expanded.filter((entry) => entry !== action.section)
          : expand(state.expanded, action.section),
      };
    }
  }
}

export function drawerSectionLabelKey(section: ProviderDrawerSection): string {
  return `chat.section_${section}`;
}
