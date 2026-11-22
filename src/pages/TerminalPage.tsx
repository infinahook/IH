import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { Eraser, Play, SquareTerminal, X } from "lucide-react";
import { api, toAppError, type AppError } from "../lib/api";
import { AnsiFilter } from "../lib/ansi";
import { useTauriEvent } from "../lib/events";
import { useAction } from "../lib/hooks";
import { useApp } from "../lib/store";
import { Badge } from "../components/Badge";
import { ConfirmDialog } from "../components/ConfirmDialog";
import { ErrorBanner } from "../components/ErrorBanner";
import { Spinner } from "../components/EmptyState";
import { Notice } from "../components/Notice";
import { PageHeader } from "../components/PageHeader";
import { ProjectGate } from "../components/ProjectGate";

interface PtyOutputEvent {
  id: string;
  data: string;
}

interface PtyExitEvent {
  id: string;
}

function trimToLines(text: string, maxLines: number): string {
  const lines = text.split("\n");
  if (lines.length <= maxLines) {
    return text;
  }
  return lines.slice(lines.length - maxLines).join("\n");
}

function measure(surface: HTMLDivElement | null): { cols: number; rows: number } {
  const width = surface?.clientWidth ?? 800;
  const height = surface?.clientHeight ?? 400;
  return {
    cols: Math.max(20, Math.min(400, Math.floor((width - 16) / 7.8))),
    rows: Math.max(5, Math.min(200, Math.floor((height - 16) / 17))),
  };
}

export function TerminalPage() {
  const { t, config, project } = useApp();
  const [terminalId, setTerminalId] = useState<string | null>(null);
  const [output, setOutput] = useState("");
  const [size, setSize] = useState({ cols: 80, rows: 24 });
  const [exited, setExited] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [confirmClose, setConfirmClose] = useState(false);
  const [writeError, setWriteError] = useState<AppError | null>(null);
  const [resizeError, setResizeError] = useState<AppError | null>(null);
  const openAction = useAction();
  const closeAction = useAction();
  const surfaceRef = useRef<HTMLDivElement>(null);
  const terminalIdRef = useRef<string | null>(null);
  const filterRef = useRef(new AnsiFilter());
  const pendingRef = useRef("");
  const frameRef = useRef<number | null>(null);
  const resizeTimerRef = useRef<number | null>(null);
  const lastSizeRef = useRef({ cols: 0, rows: 0 });
  const pinnedRef = useRef(true);
  const scrollback = Math.max(100, config?.resources.terminal_scrollback_lines ?? 5_000);

  terminalIdRef.current = terminalId;

  useEffect(() => {
    return () => {
      if (frameRef.current !== null) {
        window.cancelAnimationFrame(frameRef.current);
        frameRef.current = null;
      }
      if (terminalIdRef.current) {
        void api.terminal.close(terminalIdRef.current).catch((reason: unknown) => {
          console.error("Fermeture du terminal impossible :", reason);
        });
      }
    };
  }, []);

  useEffect(() => {
    if (!terminalId || typeof ResizeObserver === "undefined") {
      return undefined;
    }
    const surface = surfaceRef.current;
    if (!surface) {
      return undefined;
    }
    const observer = new ResizeObserver(() => {
      if (resizeTimerRef.current !== null) {
        window.clearTimeout(resizeTimerRef.current);
      }
      resizeTimerRef.current = window.setTimeout(() => {
        const next = measure(surface);
        if (lastSizeRef.current.cols === next.cols && lastSizeRef.current.rows === next.rows) {
          return;
        }
        lastSizeRef.current = next;
        setSize(next);
        void api.terminal.resize(terminalId, next.cols, next.rows).catch((reason: unknown) => {
          setResizeError(toAppError(reason));
        });
      }, 120);
    });
    observer.observe(surface);
    return () => {
      observer.disconnect();
      if (resizeTimerRef.current !== null) {
        window.clearTimeout(resizeTimerRef.current);
      }
    };
  }, [terminalId]);

  useEffect(() => {
    const surface = surfaceRef.current;
    if (surface && pinnedRef.current) {
      surface.scrollTop = surface.scrollHeight;
    }
  }, [output]);

  const scheduleFlush = () => {
    if (frameRef.current !== null) {
      return;
    }
    frameRef.current = window.requestAnimationFrame(() => {
      frameRef.current = null;
      const chunk = pendingRef.current;
      pendingRef.current = "";
      if (chunk.length > 0) {
        setOutput((previous) => trimToLines(previous + chunk, scrollback));
      }
    });
  };

  useEffect(() => {
    if (!terminalId) {
      return;
    }
    const previous = terminalId;
    if (frameRef.current !== null) {
      window.cancelAnimationFrame(frameRef.current);
      frameRef.current = null;
    }
    pendingRef.current = "";
    filterRef.current.reset();
    setTerminalId(null);
    setOutput("");
    setExited(false);
    void api.terminal.close(previous).catch((reason: unknown) => {
      setWriteError(toAppError(reason));
    });
    // Fermeture volontairement déclenchée uniquement par un changement de projet.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [project?.id]);

  const outputError = useTauriEvent<PtyOutputEvent>(
    "ih://pty-output",
    (payload) => {
      if (payload.id !== terminalId) {
        return;
      }
      const text = filterRef.current.push(payload.data);
      if (text.length > 0) {
        pendingRef.current += text;
        scheduleFlush();
      }
    },
    terminalId !== null,
  );

  const exitError = useTauriEvent<PtyExitEvent>(
    "ih://pty-exit",
    (payload) => {
      if (payload.id !== terminalId) {
        return;
      }
      const tail = filterRef.current.flush();
      if (tail.length > 0) {
        pendingRef.current += tail;
        scheduleFlush();
      }
      setExited(true);
      setMessage(t("terminal.exited"));
    },
    terminalId !== null,
  );

  const openTerminal = async () => {
    if (!project) {
      return;
    }
    setMessage(null);
    setWriteError(null);
    setResizeError(null);
    const next = measure(surfaceRef.current);
    const id = await openAction.run(() => api.terminal.open(project.id, next.cols, next.rows));
    if (id) {
      if (frameRef.current !== null) {
        window.cancelAnimationFrame(frameRef.current);
        frameRef.current = null;
      }
      filterRef.current.reset();
      pendingRef.current = "";
      setOutput("");
      setExited(false);
      setSize(next);
      lastSizeRef.current = next;
      pinnedRef.current = true;
      setTerminalId(id);
    }
  };

  const closeTerminal = async () => {
    if (!terminalId) {
      return;
    }
    const done = await closeAction.run(async () => {
      await api.terminal.close(terminalId);
      return true;
    });
    if (done) {
      if (frameRef.current !== null) {
        window.cancelAnimationFrame(frameRef.current);
        frameRef.current = null;
      }
      pendingRef.current = "";
      setTerminalId(null);
      setExited(false);
      setOutput("");
      filterRef.current.reset();
      setConfirmClose(false);
      setMessage(t("terminal.closed"));
    }
  };

  const send = (data: string) => {
    if (!terminalId || exited) {
      return;
    }
    setWriteError(null);
    void api.terminal.write(terminalId, data).catch((reason: unknown) => {
      setWriteError(toAppError(reason));
    });
  };

  const handleKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (!terminalId || exited) {
      return;
    }
    if (event.key === "Tab" && event.shiftKey) {
      return;
    }
    if (event.ctrlKey && event.shiftKey && ["v", "c"].includes(event.key.toLowerCase())) {
      return;
    }
    if (event.ctrlKey && !event.shiftKey && event.key.toLowerCase() === "v") {
      return;
    }
    if (event.ctrlKey && event.key.length === 1) {
      const letter = event.key.toUpperCase();
      if (letter >= "A" && letter <= "Z") {
        event.preventDefault();
        send(String.fromCharCode(letter.charCodeAt(0) - 64));
        return;
      }
    }
    if (event.altKey && event.key.length === 1) {
      event.preventDefault();
      send(`\u001b${event.key}`);
      return;
    }
    const sequences: Record<string, string> = {
      Enter: "\r",
      Backspace: "\u007f",
      Tab: "\t",
      Escape: "\u001b",
      ArrowUp: "\u001b[A",
      ArrowDown: "\u001b[B",
      ArrowRight: "\u001b[C",
      ArrowLeft: "\u001b[D",
      Home: "\u001b[H",
      End: "\u001b[F",
      Delete: "\u001b[3~",
      PageUp: "\u001b[5~",
      PageDown: "\u001b[6~",
    };
    const sequence = sequences[event.key];
    if (sequence) {
      event.preventDefault();
      send(sequence);
      return;
    }
    if (event.key.length === 1) {
      event.preventDefault();
      send(event.key);
    }
  };

  if (!project) {
    return (
      <>
        <PageHeader title={t("terminal.title")} />
        <ProjectGate title={t("terminal.not_running")} />
      </>
    );
  }

  return (
    <>
      <PageHeader
        title={t("terminal.title")}
        subtitle={project.path}
        actions={
          terminalId ? (
            <>
              <Badge tone={exited ? "neutral" : "success"}>
                {exited ? t("terminal.exited") : t("terminal.session_active")}
              </Badge>
              <span className="mono small">
                {t("terminal.cols_rows", { cols: size.cols, rows: size.rows })}
              </span>
              <button
                type="button"
                className="ghost"
                onClick={() => setOutput("")}
                disabled={output.length === 0}
              >
                <Eraser size={15} aria-hidden="true" />
                {t("terminal.clear_display")}
              </button>
              <button
                type="button"
                className="danger"
                onClick={() => setConfirmClose(true)}
                disabled={closeAction.busy}
              >
                <X size={15} aria-hidden="true" />
                {t("terminal.kill")}
              </button>
            </>
          ) : (
            <button
              type="button"
              className="primary"
              onClick={() => void openTerminal()}
              disabled={openAction.busy}
              aria-busy={openAction.busy}
            >
              {openAction.busy ? <Spinner /> : <Play size={15} aria-hidden="true" />}
              {t("terminal.new")}
            </button>
          )
        }
      />

      {openAction.error ? <ErrorBanner error={openAction.error} /> : null}
      {closeAction.error ? <ErrorBanner error={closeAction.error} /> : null}
      {writeError ? <ErrorBanner error={writeError} /> : null}
      {resizeError ? <ErrorBanner error={resizeError} /> : null}
      {outputError ? <ErrorBanner error={outputError} /> : null}
      {exitError ? <ErrorBanner error={exitError} /> : null}
      {message ? <Notice tone="info">{message}</Notice> : null}

      {!terminalId ? (
        <div className="terminal-idle">
          <SquareTerminal size={26} aria-hidden="true" />
          <h2>{t("terminal.not_running")}</h2>
          <p>{t("terminal.not_running_hint")}</p>
          <p className="mono small">
            {t("terminal.shell")} : {config?.terminal.shell ?? "—"}
          </p>
          <button type="button" className="primary" onClick={() => void openTerminal()}>
            {t("terminal.new")}
          </button>
        </div>
      ) : (
        <>
          <div
            className={exited ? "terminal-frame exited" : "terminal-frame"}
            ref={surfaceRef}
            tabIndex={0}
            role="application"
            aria-label={t("terminal.input_label")}
            onKeyDown={handleKeyDown}
            onMouseDown={() => surfaceRef.current?.focus()}
            onScroll={() => {
              const surface = surfaceRef.current;
              if (surface) {
                pinnedRef.current =
                  surface.scrollHeight - surface.scrollTop - surface.clientHeight < 40;
              }
            }}
            onPaste={(event) => {
              event.preventDefault();
              send(event.clipboardData.getData("text"));
            }}
          >
            <pre className="terminal-output" role="log" aria-label={t("terminal.output_label")}>
              {output.length === 0 ? (
                <span className="terminal-placeholder">{t("terminal.command_hint")}</span>
              ) : (
                output
              )}
            </pre>
          </div>
          <div className="terminal-footer">
            <span className="small muted">{t("terminal.focus_hint")}</span>
            <span className="small muted">
              {t("terminal.scrollback_hint", { lines: scrollback })}
            </span>
          </div>
          {exited ? <Notice tone="warning">{t("terminal.exited_hint")}</Notice> : null}
        </>
      )}

      <ConfirmDialog
        open={confirmClose}
        title={t("terminal.close_confirm_title")}
        description={<p>{t("terminal.close_confirm_text")}</p>}
        confirmLabel={t("terminal.kill")}
        onCancel={() => setConfirmClose(false)}
        onConfirm={() => void closeTerminal()}
        busy={closeAction.busy}
        danger
      />
    </>
  );
}
