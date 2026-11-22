import { useState } from "react";
import { Check, Copy } from "lucide-react";
import { formatJson } from "../lib/format";
import { useApp } from "../lib/store";

export function JsonView({ value, maxHeight }: { value: unknown; maxHeight?: number }) {
  const { t } = useApp();
  const [copied, setCopied] = useState(false);
  const [copyFailed, setCopyFailed] = useState(false);
  const text = formatJson(value);
  return (
    <div className="json-view">
      <div className="json-toolbar">
        <button
          type="button"
          className="ghost small"
          onClick={() => {
            setCopyFailed(false);
            if (!navigator.clipboard) {
              setCopyFailed(true);
              return;
            }
            navigator.clipboard.writeText(text).then(
              () => {
                setCopied(true);
                window.setTimeout(() => setCopied(false), 1_500);
              },
              () => setCopyFailed(true),
            );
          }}
        >
          {copied ? <Check size={13} aria-hidden="true" /> : <Copy size={13} aria-hidden="true" />}
          {copied ? t("common.copied") : t("common.copy")}
        </button>
        {copyFailed ? (
          <span className="field-error" role="alert">
            {t("common.copy_failed")}
          </span>
        ) : null}
      </div>
      <pre className="code-block" style={maxHeight ? { maxHeight } : undefined}>
        {text}
      </pre>
    </div>
  );
}
