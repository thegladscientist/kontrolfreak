import React from 'react';
import { X } from 'lucide-react';
import { useWorkspaceStore } from '../store/workspaceStore';

const ACCENT_STYLES: Record<string, string> = {
  amber: 'border-amber-500/50 bg-amber-950/70 text-amber-100',
  cyan: 'border-cyan-500/50 bg-cyan-950/70 text-cyan-100',
  violet: 'border-violet-500/50 bg-violet-950/70 text-violet-100',
  emerald: 'border-emerald-500/50 bg-emerald-950/70 text-emerald-100',
  rose: 'border-rose-500/50 bg-rose-950/70 text-rose-100',
};

/**
 * Non-modal top-right toast pill host. Used for M1 notifications and ready as the M3 hot-plug
 * "New USB HID device detected — Learn controls?" non-blocking pill seam.
 */
export const ToastHost: React.FC = () => {
  const toasts = useWorkspaceStore((s) => s.toasts);
  const dismissToast = useWorkspaceStore((s) => s.dismissToast);

  if (toasts.length === 0) return null;

  return (
    <div className="fixed top-14 right-4 z-50 flex flex-col gap-2 max-w-xs pointer-events-none">
      {toasts.map((t) => {
        const cls = ACCENT_STYLES[t.accent ?? 'amber'];
        return (
          <div
            key={t.id}
            className={`pointer-events-auto rounded-lg border px-3 py-2 shadow-xl backdrop-blur-md flex items-start justify-between gap-2 ${cls}`}
          >
            <div className="space-y-0.5 min-w-0">
              <div className="text-xs font-semibold leading-tight">{t.title}</div>
              {t.description && (
                <div className="text-[11px] opacity-85 leading-snug">{t.description}</div>
              )}
              {t.actionLabel && t.onAction && (
                <button
                  onClick={() => {
                    t.onAction?.();
                    dismissToast(t.id);
                  }}
                  className="mt-1 inline-flex px-2 py-0.5 rounded bg-white/15 hover:bg-white/25 text-[11px] font-medium"
                >
                  {t.actionLabel}
                </button>
              )}
            </div>
            <button
              onClick={() => dismissToast(t.id)}
              className="opacity-70 hover:opacity-100 p-0.5"
              aria-label="Dismiss notification"
            >
              <X className="w-3.5 h-3.5" />
            </button>
          </div>
        );
      })}
    </div>
  );
};
