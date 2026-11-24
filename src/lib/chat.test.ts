import { describe, expect, it } from "vitest";
import type { ChatOutcome, Thread } from "./api";
import {
  INITIAL_CHAT_STREAM,
  chatStreamReducer,
  displayThreadTitle,
  filterThreads,
  formatRelativeTime,
  maskSensitiveArguments,
  messageFromOutcome,
  readMessageMetadata,
  sortThreadsByUpdated,
  totalTokens,
  type ChatStreamState,
} from "./chat";

function outcome(text: string, overrides: Partial<ChatOutcome> = {}): ChatOutcome {
  return {
    user_message_id: "user-1",
    assistant_message_id: "assistant-1",
    text,
    usage: { input_tokens: 11, output_tokens: 7 },
    cancelled: false,
    finish_reason: "stop",
    elapsed_ms: 42,
    ...overrides,
  };
}

function thread(
  id: string,
  projectId: string,
  title: string,
  updatedAt: string,
  createdAt = updatedAt,
): Thread {
  return {
    id,
    project_id: projectId,
    title,
    agent_id: null,
    created_at: createdAt,
    updated_at: updatedAt,
  };
}

describe("réduction du flux de chat", () => {
  it("accumule les deltas puis remplace le texte provisoire par le texte final", () => {
    let state: ChatStreamState = chatStreamReducer(INITIAL_CHAT_STREAM, {
      type: "begin",
      requestId: "req-1",
      threadId: "thread-1",
    });
    state = chatStreamReducer(state, {
      type: "delta",
      requestId: "req-1",
      threadId: "thread-1",
      delta: "Bon",
    });
    state = chatStreamReducer(state, {
      type: "delta",
      requestId: "req-1",
      threadId: "thread-1",
      delta: "jour",
    });
    expect(state.text).toBe("Bonjour");
    expect(state.status).toBe("streaming");

    const final = outcome("Bonjour, monde");
    state = chatStreamReducer(state, {
      type: "done",
      requestId: "req-1",
      threadId: "thread-1",
      outcome: final,
    });

    expect(state.status).toBe("done");
    expect(state.text).toBe("Bonjour, monde");
    expect(state.outcome).toEqual(final);
    expect(state.text).not.toBe("Bonjour" + "Bonjour, monde");
  });

  it("ignore un delta provenant d'un autre request_id", () => {
    let state = chatStreamReducer(INITIAL_CHAT_STREAM, {
      type: "begin",
      requestId: "req-courant",
      threadId: "thread-1",
    });
    state = chatStreamReducer(state, {
      type: "delta",
      requestId: "req-courant",
      threadId: "thread-1",
      delta: "A",
    });
    state = chatStreamReducer(state, {
      type: "delta",
      requestId: "req-ancien",
      threadId: "thread-1",
      delta: "B",
    });
    expect(state.text).toBe("A");
    expect(state.requestId).toBe("req-courant");
  });

  it("ignore un delta, un done ou une erreur tardifs après la fin", () => {
    let state = chatStreamReducer(INITIAL_CHAT_STREAM, {
      type: "begin",
      requestId: "req-1",
      threadId: "thread-1",
    });
    state = chatStreamReducer(state, {
      type: "done",
      requestId: "req-1",
      threadId: "thread-1",
      outcome: outcome("final"),
    });
    const afterDone = state;
    state = chatStreamReducer(state, {
      type: "delta",
      requestId: "req-1",
      threadId: "thread-1",
      delta: "tardif",
    });
    expect(state).toBe(afterDone);
    state = chatStreamReducer(state, {
      type: "error",
      requestId: "req-1",
      threadId: "thread-1",
      code: "network_error",
      message: "trop tard",
    });
    expect(state).toBe(afterDone);
  });

  it("réinitialise le texte provisoire après une erreur, puis revient à l'état initial", () => {
    let state = chatStreamReducer(INITIAL_CHAT_STREAM, {
      type: "begin",
      requestId: "req-1",
      threadId: "thread-1",
    });
    state = chatStreamReducer(state, {
      type: "delta",
      requestId: "req-1",
      threadId: "thread-1",
      delta: "partiel",
    });
    state = chatStreamReducer(state, {
      type: "error",
      requestId: "req-1",
      threadId: "thread-1",
      code: "provider_error",
      message: "clé absente",
    });

    expect(state.status).toBe("error");
    expect(state.text).toBe("");
    expect(state.error).toEqual({ code: "provider_error", message: "clé absente" });
    expect(state.outcome).toBeNull();

    state = chatStreamReducer(state, { type: "reset" });
    expect(state).toEqual(INITIAL_CHAT_STREAM);
    expect(state.status).toBe("idle");
  });
});

describe("titre des conversations", () => {
  it("localise le titre sentinelle reconnu par le noyau", () => {
    expect(displayThreadTitle("Nouvelle conversation", "New conversation")).toBe(
      "New conversation",
    );
    expect(displayThreadTitle("  ", "Nouvelle conversation")).toBe("Nouvelle conversation");
    expect(displayThreadTitle("Titre réel", "New conversation")).toBe("Titre réel");
  });
});

describe("liste des conversations", () => {
  const threads = [
    thread("t-1", "p-1", "Ancienne", "2026-01-01T10:00:00.000Z"),
    thread("t-2", "p-1", "Récente", "2026-03-01T10:00:00.000Z"),
    thread("t-3", "p-2", "Autre projet", "2026-04-01T10:00:00.000Z"),
    thread("t-4", "p-1", "Recherche streaming", "2026-02-01T10:00:00.000Z"),
  ];

  it("filtre par projet actif puis par titre", () => {
    const projectThreads = filterThreads(threads, "p-1");
    expect(projectThreads.map((entry) => entry.id)).toEqual(["t-1", "t-2", "t-4"]);
    expect(filterThreads(threads, "p-1", "stream").map((entry) => entry.id)).toEqual([
      "t-4",
    ]);
    expect(filterThreads(threads, "p-1", "absente")).toEqual([]);
    expect(filterThreads(threads, "p-1", "  RéCeNtE  ").map((entry) => entry.id)).toEqual([
      "t-2",
    ]);
  });

  it("trie les conversations par updated_at décroissant", () => {
    expect(sortThreadsByUpdated(filterThreads(threads, "p-1")).map((entry) => entry.id)).toEqual([
      "t-2",
      "t-4",
      "t-1",
    ]);
    // Le tri ne modifie pas le tableau d'origine.
    expect(threads[0].id).toBe("t-1");
  });
});

describe("métadonnées et usage", () => {
  it("lit l'usage, l'annulation et la raison de fin sans jeter sur du JSON malformé", () => {
    const metadata = readMessageMetadata({
      usage: { input_tokens: 12, output_tokens: 4 },
      cancelled: true,
      finish_reason: "stop",
      elapsed_ms: 123,
      provider: "ollama",
      model: "llama3.2",
    });
    expect(metadata.usage).toEqual({ input_tokens: 12, output_tokens: 4 });
    expect(metadata.cancelled).toBe(true);
    expect(metadata.elapsedMs).toBe(123);
    expect(totalTokens(metadata.usage)).toBe(16);

    const broken = readMessageMetadata({ usage: "n/a", cancelled: "oui", elapsed_ms: "x" });
    expect(broken.usage).toBeNull();
    expect(broken.cancelled).toBe(false);
    expect(broken.elapsedMs).toBeNull();
    expect(totalTokens(null)).toBeNull();
  });

  it("construit le message final avec l'usage et l'annulation", () => {
    const message = messageFromOutcome(
      outcome("Merci", { cancelled: true, finish_reason: null, elapsed_ms: 99 }),
      "thread-1",
      "ollama",
      "llama3.2",
      "2026-06-01T12:00:00.000Z",
    );
    expect(message.id).toBe("assistant-1");
    expect(message.role).toBe("assistant");
    expect(message.content).toBe("Merci");
    expect(message.created_at).toBe("2026-06-01T12:00:00.000Z");
    const metadata = readMessageMetadata(message.metadata);
    expect(metadata.cancelled).toBe(true);
    expect(metadata.provider).toBe("ollama");
    expect(metadata.usage).toEqual({ input_tokens: 11, output_tokens: 7 });
  });

  it("masque les arguments sensibles des cartes d'outil", () => {
    const masked = maskSensitiveArguments({
      text: "bonjour",
      api_key: "sk-secret",
      nested: { password: "motdepasse", bearer: "jeton", keep: 1 },
      list: [{ token: "abc" }],
    });
    expect(masked).toEqual({
      text: "bonjour",
      api_key: "[masqué]",
      nested: { password: "[masqué]", bearer: "[masqué]", keep: 1 },
      list: [{ token: "[masqué]" }],
    });
    expect(maskSensitiveArguments("texte")).toBe("texte");
    expect(maskSensitiveArguments(null)).toBeNull();
  });

  it("formate une date relative réelle", () => {
    const now = Date.parse("2026-06-01T12:00:00.000Z");
    expect(formatRelativeTime("2026-06-01T11:57:00.000Z", "fr", now)).toContain("3");
    expect(formatRelativeTime("2026-06-01T11:57:00.000Z", "en", now)).toContain("3");
    expect(formatRelativeTime(null, "fr", now)).toBe("—");
    expect(formatRelativeTime("date-invalide", "fr", now)).toBe("—");
  });
});
