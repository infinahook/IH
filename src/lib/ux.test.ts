import { describe, expect, it } from "vitest";
import {
  DEFAULT_LAYOUT,
  LAYOUT_UX_VERSION,
  MIN_CONVERSATION_WIDTH,
  MIN_WORKSPACE_WIDTH,
  clampLayout,
  defaultConversationWidth,
  defaultWorkspaceLayout,
  isCompactWidth,
  migrateLayout,
  type LayoutBounds,
} from "./layout";
import type { Message } from "./api";
import {
  INITIAL_PROVIDER_DRAWER_STATE,
  changeProviderSelection,
  providerAvailability,
  providerDrawerReducer,
  type ProviderAvailabilitySource,
  type ProviderDrawerSection,
} from "./ux";

const BOUNDS: LayoutBounds = { width: 1440, height: 900 };

function provider(
  overrides: Partial<ProviderAvailabilitySource> = {},
): ProviderAvailabilitySource {
  return {
    enabled: true,
    kind: "openai_compatible",
    has_secret: true,
    secret_stored: true,
    default_model: "gpt-test",
    models: [],
    ...overrides,
  };
}

describe("répartition par défaut de la largeur", () => {
  it("donne au moins 55 % à la conversation et replie l'explorateur", () => {
    const layout = defaultWorkspaceLayout(BOUNDS);
    expect(layout.conversation_width / BOUNDS.width).toBeGreaterThanOrEqual(0.55);
    expect(layout.explorer_visible).toBe(false);
    expect(layout.conversation_width).toBe(defaultConversationWidth(BOUNDS));
    expect(layout.ux_version).toBe(LAYOUT_UX_VERSION);
  });

  it("reste au-dessus de 55 % et sous la borne éditeur à 1280/1440/1920", () => {
    for (const width of [1280, 1440, 1920]) {
      const bounds = { width, height: 900 };
      const layout = defaultWorkspaceLayout(bounds);
      expect(layout.conversation_width / width).toBeGreaterThanOrEqual(0.55);
      expect(layout.conversation_width + MIN_WORKSPACE_WIDTH).toBeLessThanOrEqual(width);
      expect(layout.explorer_visible).toBe(false);
    }
  });

  it("migre une disposition historique une seule fois", () => {
    const legacy = {
      ...DEFAULT_LAYOUT,
      ux_version: 0,
      conversation_width: 330,
      explorer_visible: true,
      tabs: ["src/App.tsx"],
      active_tab: "src/App.tsx",
    };
    const migrated = migrateLayout(legacy, BOUNDS);
    expect(migrated.ux_version).toBe(LAYOUT_UX_VERSION);
    expect(migrated.explorer_visible).toBe(false);
    expect(migrated.conversation_width).toBeGreaterThanOrEqual(0.55 * BOUNDS.width);
    expect(migrated.tabs).toEqual(["src/App.tsx"]);
    expect(migrated.active_tab).toBe("src/App.tsx");
    // Deuxième passage : la largeur choisie par l'utilisateur n'est plus touchée.
    const resized = { ...migrated, conversation_width: 700 };
    expect(migrateLayout(resized, BOUNDS)).toBe(resized);
  });
});

describe("bornes et mode compact", () => {
  it("borne la conversation et l'explorateur sans écraser le workspace", () => {
    const bounds = { width: 1280, height: 800 };
    const clamped = clampLayout(
      {
        ...DEFAULT_LAYOUT,
        conversation_width: 99999,
        explorer_width: 99999,
        explorer_visible: true,
      },
      bounds,
    );
    expect(clamped.conversation_width).toBeGreaterThanOrEqual(MIN_CONVERSATION_WIDTH);
    expect(
      clamped.conversation_width + clamped.explorer_width,
    ).toBeLessThanOrEqual(bounds.width - MIN_WORKSPACE_WIDTH - 14);
    expect(clampLayout({ ...DEFAULT_LAYOUT, conversation_width: 10 }, bounds).conversation_width)
      .toBe(MIN_CONVERSATION_WIDTH);
  });

  it("passe en mode compact sous 1200 px", () => {
    expect(isCompactWidth(1199)).toBe(true);
    expect(isCompactWidth(1200)).toBe(false);
    expect(isCompactWidth(1920)).toBe(false);
    expect(isCompactWidth(Number.NaN)).toBe(true);
  });
});

describe("état de disponibilité de la barre de conversation", () => {
  it("est vert quand le fournisseur est actif, le modèle choisi et la clé stockée", () => {
    const state = providerAvailability({ provider: provider(), model: "gpt-test" });
    expect(state.tone).toBe("success");
    expect(state.ready).toBe(true);
    expect(state.labelKey).toBe("chat.status_ready");
  });

  it("est orange quand la clé requise manque", () => {
    const state = providerAvailability({
      provider: provider({ has_secret: false, secret_stored: false }),
      model: "gpt-test",
    });
    expect(state.tone).toBe("warning");
    expect(state.ready).toBe(false);
    expect(state.labelKey).toBe("chat.status_missing_key");
    expect(state.requiresSecret).toBe(true);
  });

  it("est orange quand aucun modèle n'est choisi", () => {
    const state = providerAvailability({ provider: provider(), model: "  " });
    expect(state.tone).toBe("warning");
    expect(state.labelKey).toBe("chat.status_no_model");
    expect(state.ready).toBe(false);
  });

  it("est gris pour un fournisseur désactivé ou absent", () => {
    expect(
      providerAvailability({ provider: provider({ enabled: false }), model: "gpt-test" }).tone,
    ).toBe("neutral");
    expect(providerAvailability({ provider: null, model: "" }).tone).toBe("neutral");
    expect(providerAvailability({ provider: null, model: "" }).labelKey).toBe(
      "chat.status_no_provider",
    );
  });

  it("est rouge en cas d'erreur et vert pour Ollama sans clé", () => {
    expect(
      providerAvailability({ provider: provider(), model: "gpt-test", error: true }).tone,
    ).toBe("danger");
    const ollama = providerAvailability({
      provider: provider({
        kind: "ollama",
        has_secret: false,
        secret_stored: false,
        default_model: "llama",
      }),
      model: "llama",
    });
    expect(ollama.tone).toBe("success");
    expect(ollama.requiresSecret).toBe(false);
  });
});

describe("drawer de configuration", () => {
  it("s'ouvre, se ferme et garde une section active", () => {
    let state = INITIAL_PROVIDER_DRAWER_STATE;
    expect(state.open).toBe(false);

    state = providerDrawerReducer(state, { type: "open" });
    expect(state.open).toBe(true);

    const section: ProviderDrawerSection = "keys";
    state = providerDrawerReducer(state, { type: "select_section", section });
    expect(state.section).toBe("keys");
    expect(state.expanded).toContain("keys");

    state = providerDrawerReducer(state, { type: "toggle_section", section });
    expect(state.expanded).not.toContain("keys");
    expect(state.section).toBe("keys");

    state = providerDrawerReducer(state, { type: "close" });
    expect(state.open).toBe(false);
    expect(state.section).toBe("keys");
  });
});

describe("changement de fournisseur", () => {
  it("ne réinitialise ni le fil ni les messages", () => {
    const messages: readonly Message[] = Object.freeze([
      {
        id: "m1",
        thread_id: "thread-1",
        role: "user",
        content: "Bonjour",
        metadata: null,
        created_at: "2026-01-01T00:00:00Z",
      },
    ]);
    const state = {
      providerId: "openai",
      model: "gpt-test",
      threadId: "thread-1",
      messages,
    };
    const next = changeProviderSelection(state, {
      id: "anthropic",
      default_model: null,
      models: [{ id: "claude-test" }],
    });
    expect(next.providerId).toBe("anthropic");
    expect(next.model).toBe("claude-test");
    expect(next.threadId).toBe("thread-1");
    expect(next.messages).toBe(messages);
    expect(next.messages).toHaveLength(1);
  });

  it("garde la conversation quand le fournisseur disparaît", () => {
    const messages: readonly Message[] = [];
    const next = changeProviderSelection(
      { providerId: "openai", model: "gpt-test", threadId: "thread-1", messages },
      null,
    );
    expect(next.providerId).toBe("");
    expect(next.model).toBe("");
    expect(next.threadId).toBe("thread-1");
    expect(next.messages).toBe(messages);
  });
});
