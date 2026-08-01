import * as React from "react";
import { cn } from "@irixmail/shared";

export function SectionHeader({
  title,
  description,
}: {
  title: React.ReactNode;
  description?: React.ReactNode;
}) {
  return (
    <div className="space-y-1">
      <h1 className="text-lg font-semibold tracking-tight">{title}</h1>
      {description ? <p className="text-[13px] text-muted-foreground">{description}</p> : null}
    </div>
  );
}

export function SettingsCard({
  title,
  description,
  action,
  footer,
  bodyClassName,
  className,
  children,
}: {
  title?: React.ReactNode;
  description?: React.ReactNode;
  action?: React.ReactNode;
  footer?: React.ReactNode;
  bodyClassName?: string;
  className?: string;
  children?: React.ReactNode;
}) {
  return (
    <section className={cn("rounded-lg border bg-card", className)}>
      {title ? (
        <header className="flex items-start justify-between gap-3 border-b px-4 py-3">
          <div className="space-y-0.5">
            <h2 className="text-[13px] font-semibold">{title}</h2>
            {description ? <p className="text-xs text-muted-foreground">{description}</p> : null}
          </div>
          {action}
        </header>
      ) : null}
      {children ? <div className={cn("p-4", bodyClassName)}>{children}</div> : null}
      {footer ? <footer className="flex justify-end gap-2 border-t px-4 py-3">{footer}</footer> : null}
    </section>
  );
}

export function SettingsRow({
  label,
  hint,
  htmlFor,
  children,
}: {
  label: React.ReactNode;
  hint?: React.ReactNode;
  htmlFor?: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex items-center justify-between gap-6">
      <div className="space-y-0.5">
        <label htmlFor={htmlFor} className="text-[13px] font-medium">
          {label}
        </label>
        {hint ? <p className="text-xs text-muted-foreground">{hint}</p> : null}
      </div>
      {children}
    </div>
  );
}
