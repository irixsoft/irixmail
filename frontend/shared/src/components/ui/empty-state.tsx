import * as React from "react";
import { type LucideIcon } from "lucide-react";

import { cn } from "../../lib/utils";

export interface EmptyStateProps extends Omit<React.ComponentProps<"div">, "title"> {
  icon?: LucideIcon;
  title: React.ReactNode;
  description?: React.ReactNode;
  action?: React.ReactNode;
}

function EmptyState({
  icon: Icon,
  title,
  description,
  action,
  className,
  ...props
}: EmptyStateProps) {
  return (
    <div
      data-slot="empty-state"
      className={cn(
        "flex flex-col items-center justify-center gap-3 rounded-lg border border-dashed bg-card/40 px-6 py-12 text-center",
        className,
      )}
      {...props}
    >
      {Icon ? (
        <div className="flex size-11 items-center justify-center rounded-md border bg-muted/50 text-muted-foreground">
          <Icon className="size-5" />
        </div>
      ) : null}
      <div className="space-y-1">
        <p className="text-sm font-medium text-foreground">{title}</p>
        {description ? (
          <p className="mx-auto max-w-sm text-sm text-muted-foreground">{description}</p>
        ) : null}
      </div>
      {action ? <div className="mt-1">{action}</div> : null}
    </div>
  );
}

export { EmptyState };
