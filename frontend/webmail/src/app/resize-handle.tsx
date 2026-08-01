import * as React from "react";
import { cn } from "@irixmail/shared";

export interface ResizeHandleProps {
  value: number;
  min: number;
  max: number;
  onChange: (size: number) => void;
  onCommit: (size: number) => void;
  onReset: () => void;
  label: string;
  axis?: "x" | "y";
}

export function ResizeHandle({
  value,
  min,
  max,
  onChange,
  onCommit,
  onReset,
  label,
  axis = "x",
}: ResizeHandleProps) {
  const vertical = axis === "y";
  const dragging = React.useRef<{ start: number; size: number } | null>(null);
  const latest = React.useRef(value);
  latest.current = value;

  const clamp = React.useCallback((size: number) => Math.min(max, Math.max(min, size)), [min, max]);

  const onPointerDown = (event: React.PointerEvent<HTMLDivElement>) => {
    event.preventDefault();
    dragging.current = { start: vertical ? event.clientY : event.clientX, size: latest.current };
    const move = (moveEvent: PointerEvent) => {
      if (!dragging.current) return;
      const position = vertical ? moveEvent.clientY : moveEvent.clientX;
      onChange(clamp(dragging.current.size + position - dragging.current.start));
    };
    const up = () => {
      dragging.current = null;
      document.removeEventListener("pointermove", move);
      document.removeEventListener("pointerup", up);
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
      onCommit(latest.current);
    };
    document.addEventListener("pointermove", move);
    document.addEventListener("pointerup", up);
    document.body.style.cursor = vertical ? "row-resize" : "col-resize";
    document.body.style.userSelect = "none";
  };

  const onKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    const grow = vertical ? "ArrowDown" : "ArrowRight";
    const shrink = vertical ? "ArrowUp" : "ArrowLeft";
    if (event.key !== grow && event.key !== shrink) return;
    event.preventDefault();
    const next = clamp(latest.current + (event.key === grow ? 10 : -10));
    onChange(next);
    onCommit(next);
  };

  return (
    <div
      role="separator"
      aria-label={label}
      aria-orientation={vertical ? "horizontal" : "vertical"}
      aria-valuenow={value}
      aria-valuemin={min}
      aria-valuemax={max}
      tabIndex={0}
      onPointerDown={onPointerDown}
      onKeyDown={onKeyDown}
      onDoubleClick={onReset}
      className={cn(
        "z-10 shrink-0 bg-transparent outline-none transition-colors",
        vertical ? "-my-0.5 h-1 w-full cursor-row-resize" : "-mx-0.5 w-1 cursor-col-resize",
        "hover:bg-border focus-visible:bg-primary/50 active:bg-primary/50",
      )}
    />
  );
}
