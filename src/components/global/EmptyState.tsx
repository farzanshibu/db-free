// SOT: empty-state-component
import type { ReactNode } from "react";
import { Icon, type IconName } from "@/lib/icons";

export function EmptyState({ title, body, action, icon }: { title: string; body?: string; action?: ReactNode; icon?: IconName }) {
  return (
    <div className="flex h-full flex-col items-center justify-center gap-2 p-5 text-center">
      {icon ? (
        <span className="mb-0.5 flex size-9 items-center justify-center rounded-xl glass-card text-accent shadow-lg">
          <Icon name={icon} size={16} />
        </span>
      ) : null}
      <p className="text-[13px] font-medium text-foreground tracking-tight">{title}</p>
      {body ? <p className="max-w-sm text-xs text-muted leading-relaxed">{body}</p> : null}
      {action ? <div className="mt-1">{action}</div> : null}
    </div>
  );
}
