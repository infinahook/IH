import { useRef, type KeyboardEvent, type PointerEvent } from "react";

export type ResizerOrientation = "vertical" | "horizontal";

const STEP = 16;
const LARGE_STEP = 64;

/**
 * Séparateur déplaçable maison (aucun composant externe).
 *
 * `vertical` = barre verticale qui règle une largeur ; `horizontal` = barre
 * horizontale qui règle une hauteur. Le clamp est appliqué par l'appelant,
 * mais les bornes ARIA reflètent exactement celles du glissement.
 */
export function Resizer({
  orientation,
  value,
  min,
  max,
  label,
  onChange,
  disabled = false,
}: {
  orientation: ResizerOrientation;
  value: number;
  min: number;
  max: number;
  label: string;
  onChange: (value: number) => void;
  disabled?: boolean;
}) {
  const dragRef = useRef<{ start: number; value: number } | null>(null);

  const clamp = (next: number) => Math.min(Math.max(Math.round(next), min), max);

  const endDrag = () => {
    dragRef.current = null;
    document.body.classList.remove("resizing-column", "resizing-row");
  };

  const onPointerDown = (event: PointerEvent<HTMLDivElement>) => {
    if (disabled) {
      return;
    }
    event.preventDefault();
    dragRef.current = {
      start: orientation === "vertical" ? event.clientX : event.clientY,
      value,
    };
    document.body.classList.add(
      orientation === "vertical" ? "resizing-column" : "resizing-row",
    );
    const move = (incoming: globalThis.PointerEvent) => {
      const drag = dragRef.current;
      if (!drag) {
        return;
      }
      const position = orientation === "vertical" ? incoming.clientX : incoming.clientY;
      onChange(clamp(drag.value + (position - drag.start)));
    };
    const stop = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", stop);
      window.removeEventListener("pointercancel", stop);
      endDrag();
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", stop);
    window.addEventListener("pointercancel", stop);
  };

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (disabled) {
      return;
    }
    const decrease =
      orientation === "vertical"
        ? ["ArrowLeft", "ArrowDown"]
        : ["ArrowUp", "ArrowLeft"];
    const increase =
      orientation === "vertical"
        ? ["ArrowRight", "ArrowUp"]
        : ["ArrowDown", "ArrowRight"];
    const step = event.shiftKey ? LARGE_STEP : STEP;
    if (decrease.includes(event.key)) {
      event.preventDefault();
      onChange(clamp(value - step));
    } else if (increase.includes(event.key)) {
      event.preventDefault();
      onChange(clamp(value + step));
    } else if (event.key === "Home") {
      event.preventDefault();
      onChange(min);
    } else if (event.key === "End") {
      event.preventDefault();
      onChange(max);
    }
  };

  return (
    <div
      className={`resizer resizer-${orientation}${disabled ? " disabled" : ""}`}
      role="separator"
      aria-orientation={orientation}
      aria-valuemin={Math.round(min)}
      aria-valuemax={Math.round(max)}
      aria-valuenow={Math.round(value)}
      aria-label={label}
      tabIndex={disabled ? -1 : 0}
      onPointerDown={onPointerDown}
      onKeyDown={onKeyDown}
    >
      <span className="resizer-grip" aria-hidden="true" />
    </div>
  );
}
