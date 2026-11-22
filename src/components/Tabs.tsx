import { useRef, type KeyboardEvent } from "react";

export interface TabItem<T extends string> {
  id: T;
  label: string;
  detail?: string;
}

export function tabButtonId(prefix: string, id: string): string {
  return `${prefix}-tab-${id}`;
}

export function tabPanelId(prefix: string, id: string): string {
  return `${prefix}-panel-${id}`;
}

export function Tabs<T extends string>({
  idPrefix,
  label,
  tabs,
  active,
  onChange,
}: {
  idPrefix: string;
  label: string;
  tabs: readonly TabItem<T>[];
  active: T;
  onChange: (id: T) => void;
}) {
  const refs = useRef<Array<HTMLButtonElement | null>>([]);

  const move = (delta: number) => {
    const index = tabs.findIndex((tab) => tab.id === active);
    if (index < 0) {
      return;
    }
    const next = (index + delta + tabs.length) % tabs.length;
    onChange(tabs[next].id);
    refs.current[next]?.focus();
  };

  const handleKeyDown = (event: KeyboardEvent<HTMLButtonElement>) => {
    switch (event.key) {
      case "ArrowRight":
      case "ArrowDown":
        event.preventDefault();
        move(1);
        break;
      case "ArrowLeft":
      case "ArrowUp":
        event.preventDefault();
        move(-1);
        break;
      case "Home":
        event.preventDefault();
        onChange(tabs[0].id);
        refs.current[0]?.focus();
        break;
      case "End":
        event.preventDefault();
        onChange(tabs[tabs.length - 1].id);
        refs.current[tabs.length - 1]?.focus();
        break;
      default:
        break;
    }
  };

  return (
    <div className="tabs" role="tablist" aria-label={label}>
      {tabs.map((tab, index) => {
        const selected = tab.id === active;
        return (
          <button
            key={tab.id}
            ref={(element) => {
              refs.current[index] = element;
            }}
            type="button"
            role="tab"
            id={tabButtonId(idPrefix, tab.id)}
            aria-selected={selected}
            aria-controls={tabPanelId(idPrefix, tab.id)}
            tabIndex={selected ? 0 : -1}
            className={selected ? "tab active" : "tab"}
            onClick={() => onChange(tab.id)}
            onKeyDown={handleKeyDown}
          >
            <span>{tab.label}</span>
            {tab.detail ? <span className="tab-count">{tab.detail}</span> : null}
          </button>
        );
      })}
    </div>
  );
}
