import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useEffect, useRef, useState } from "react";
import { toAppError, type AppError } from "./api";

/**
 * Abonne un composant à un événement Tauri et renvoie l'erreur d'abonnement
 * éventuelle afin qu'elle puisse être affichée, jamais avalée.
 */
export function useTauriEvent<T>(
  event: string,
  handler: (payload: T) => void,
  enabled = true,
): AppError | null {
  const handlerRef = useRef(handler);
  handlerRef.current = handler;
  const [error, setError] = useState<AppError | null>(null);

  useEffect(() => {
    if (!enabled) {
      return undefined;
    }
    let unlisten: UnlistenFn | null = null;
    let cancelled = false;
    listen<T>(event, (incoming) => handlerRef.current(incoming.payload)).then(
      (stop) => {
        if (cancelled) {
          stop();
        } else {
          unlisten = stop;
          setError(null);
        }
      },
      (reason: unknown) => {
        if (!cancelled) {
          setError(toAppError(reason));
        }
      },
    );
    return () => {
      cancelled = true;
      if (unlisten) {
        unlisten();
      }
    };
  }, [event, enabled]);

  return error;
}
