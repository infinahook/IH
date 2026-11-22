import type { ReactNode } from "react";
import { Spinner } from "./EmptyState";
import { Modal } from "./Modal";
import { useApp } from "../lib/store";

export function ConfirmDialog({
  open,
  title,
  description,
  confirmLabel,
  onConfirm,
  onCancel,
  busy = false,
  danger = false,
  children,
}: {
  open: boolean;
  title: string;
  description: ReactNode;
  confirmLabel?: string;
  onConfirm: () => void;
  onCancel: () => void;
  busy?: boolean;
  danger?: boolean;
  children?: ReactNode;
}) {
  const { t } = useApp();
  return (
    <Modal
      open={open}
      title={title}
      onClose={onCancel}
      footer={
        <>
          <button type="button" className="ghost" onClick={onCancel} disabled={busy}>
            {t("common.cancel")}
          </button>
          <button
            type="button"
            className={danger ? "danger" : "primary"}
            onClick={onConfirm}
            disabled={busy}
            aria-busy={busy}
          >
            {busy ? <Spinner /> : null}
            {confirmLabel ?? t("common.confirm")}
          </button>
        </>
      }
    >
      <div className="confirm-description">{description}</div>
      {children}
    </Modal>
  );
}
