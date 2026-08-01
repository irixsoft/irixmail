import * as React from "react";
import { cn } from "@irixmail/shared";

import { formatTime, instanceStart } from "./display";
import type { EventInstance } from "./types";

export function chipVars(hex: string): React.CSSProperties {
  return { "--chip": hex } as React.CSSProperties;
}

function cancelled(instance: EventInstance): boolean {
  return instance.event.status === "cancelled";
}

export function EventBlock({
  instance,
  color,
  style,
  onSelect,
  onMoveStart,
  onResizeStart,
  dragging,
}: {
  instance: EventInstance;
  color: string;
  style: React.CSSProperties;
  onSelect: (instance: EventInstance, element: HTMLElement) => void;
  onMoveStart?: (event: React.PointerEvent<HTMLElement>, instance: EventInstance) => void;
  onResizeStart?: (event: React.PointerEvent<HTMLElement>, instance: EventInstance) => void;
  dragging?: boolean;
}) {
  const short = (typeof style.height === "number" ? style.height : 0) < 34;
  return (
    <div
      role="button"
      tabIndex={0}
      style={{ ...chipVars(color), ...style }}
      onPointerDown={(event) => {
        event.stopPropagation();
        onMoveStart?.(event, instance);
      }}
      onClick={(event) => onSelect(instance, event.currentTarget)}
      onKeyDown={(event) => {
        if (event.key === "Enter" || event.key === " ") onSelect(instance, event.currentTarget);
      }}
      className={cn(
        "cal-chip absolute z-10 flex cursor-pointer select-none flex-col overflow-hidden rounded-md px-1.5 py-0.5 text-left outline-none focus-visible:ring-2 focus-visible:ring-ring",
        short && "flex-row items-center gap-1.5 py-0",
        dragging && "opacity-70 shadow-lg",
        cancelled(instance) && "line-through opacity-60",
      )}
    >
      <span className="truncate text-[12px] font-medium leading-tight">
        {instance.event.title || "Untitled"}
      </span>
      <span className="truncate font-mono text-[10px] leading-tight text-muted-foreground">
        {formatTime(instanceStart(instance))}
      </span>
      {onResizeStart ? (
        <span
          aria-hidden
          onPointerDown={(event) => {
            event.stopPropagation();
            onResizeStart(event, instance);
          }}
          className="absolute inset-x-0 bottom-0 h-2 cursor-ns-resize"
        />
      ) : null}
    </div>
  );
}

export function EventBar({
  instance,
  color,
  style,
  onSelect,
}: {
  instance: EventInstance;
  color: string;
  style?: React.CSSProperties;
  onSelect: (instance: EventInstance, element: HTMLElement) => void;
}) {
  return (
    <button
      type="button"
      style={{ ...chipVars(color), ...style }}
      onClick={(event) => onSelect(instance, event.currentTarget)}
      className={cn(
        "cal-chip flex h-[18px] items-center gap-1.5 overflow-hidden rounded px-1.5 text-left text-[11px] font-medium leading-none",
        cancelled(instance) && "line-through opacity-60",
      )}
    >
      <span className="truncate">{instance.event.title || "Untitled"}</span>
    </button>
  );
}

export function EventChipInline({
  instance,
  color,
  onSelect,
}: {
  instance: EventInstance;
  color: string;
  onSelect: (instance: EventInstance, element: HTMLElement) => void;
}) {
  return (
    <button
      type="button"
      style={chipVars(color)}
      onClick={(event) => {
        event.stopPropagation();
        onSelect(instance, event.currentTarget);
      }}
      className={cn(
        "flex w-full items-center gap-1.5 overflow-hidden rounded px-1 py-px text-left text-[11px] leading-tight hover:bg-accent/60",
        cancelled(instance) && "line-through opacity-60",
      )}
    >
      <span className="cal-dot size-1.5 shrink-0 rounded-full" />
      <span className="shrink-0 font-mono text-[10px] tabular-nums text-muted-foreground">
        {formatTime(instanceStart(instance))}
      </span>
      <span className="truncate">{instance.event.title || "Untitled"}</span>
    </button>
  );
}
