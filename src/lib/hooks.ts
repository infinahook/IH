import { useCallback, useEffect, useRef, useState, type DependencyList } from "react";
import { toAppError, type AppError } from "./api";

export interface AsyncResult<T> {
  data: T | null;
  loading: boolean;
  error: AppError | null;
  reload: () => void;
  setData: (updater: T | null | ((current: T | null) => T | null)) => void;
}

/** Chargement asynchrone réel : aucune temporisation simulée, aucun état factice. */
export function useAsync<T>(loader: () => Promise<T>, deps: DependencyList = []): AsyncResult<T> {
  const loaderRef = useRef(loader);
  loaderRef.current = loader;
  const [state, setState] = useState<{ data: T | null; loading: boolean; error: AppError | null }>({
    data: null,
    loading: true,
    error: null,
  });
  const [tick, setTick] = useState(0);

  useEffect(() => {
    let cancelled = false;
    setState((current) => ({ ...current, loading: true, error: null }));
    loaderRef.current().then(
      (data) => {
        if (!cancelled) {
          setState({ data, loading: false, error: null });
        }
      },
      (reason: unknown) => {
        if (!cancelled) {
          setState((current) => ({ ...current, loading: false, error: toAppError(reason) }));
        }
      },
    );
    return () => {
      cancelled = true;
    };
    // Le tableau de dépendances est volontairement fourni par l'appelant.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [...deps, tick]);

  const reload = useCallback(() => setTick((value) => value + 1), []);
  const setData = useCallback(
    (updater: T | null | ((current: T | null) => T | null)) => {
      setState((current) => ({
        ...current,
        data:
          typeof updater === "function"
            ? (updater as (value: T | null) => T | null)(current.data)
            : updater,
      }));
    },
    [],
  );

  return { ...state, reload, setData };
}

export interface ActionResult {
  busy: boolean;
  error: AppError | null;
  message: string | null;
  run: <T>(task: () => Promise<T>) => Promise<T | undefined>;
  setError: (error: AppError | null) => void;
  setMessage: (message: string | null) => void;
  reset: () => void;
}

/** Encapsule une action IPC : verrouille le bouton pendant l'appel et expose l'erreur réelle. */
export function useAction(): ActionResult {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<AppError | null>(null);
  const [message, setMessage] = useState<string | null>(null);

  const run = useCallback(async <T>(task: () => Promise<T>): Promise<T | undefined> => {
    setBusy(true);
    setError(null);
    setMessage(null);
    try {
      return await task();
    } catch (reason) {
      setError(toAppError(reason));
      return undefined;
    } finally {
      setBusy(false);
    }
  }, []);

  const reset = useCallback(() => {
    setError(null);
    setMessage(null);
  }, []);

  return { busy, error, message, run, setError, setMessage, reset };
}

/** Valeur retardée, utilisée pour le redimensionnement du terminal. */
export function useDebouncedValue<T>(value: T, delayMs: number): T {
  const [debounced, setDebounced] = useState(value);
  useEffect(() => {
    const timer = window.setTimeout(() => setDebounced(value), delayMs);
    return () => window.clearTimeout(timer);
  }, [value, delayMs]);
  return debounced;
}
