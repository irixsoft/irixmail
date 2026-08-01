import * as React from "react";

import { clipToDay, columnFromX, dragCreateRange, dragMoveRange, dragResizeEnd, slotMath } from "./layout";
import type { EventInstance } from "./types";

const MOVE_THRESHOLD = 4;

export interface DragPreview {
  kind: "create" | "move" | "resize";
  dayIndex: number;
  startMinutes: number;
  endMinutes: number;
  instanceKey: string | null;
}

interface Session {
  kind: "create" | "move" | "resize";
  instance: EventInstance | null;
  dayIndex: number;
  anchorMinutes: number;
  startMinutes: number;
  durationMinutes: number;
  grabOffset: number;
  originX: number;
  originY: number;
  active: boolean;
}

export interface TimeGridDragOptions {
  containerRef: React.RefObject<HTMLDivElement | null>;
  days: Date[];
  enabled: boolean;
  onCreate: (dayIndex: number, startMinutes: number, endMinutes: number) => void;
  onMove: (instance: EventInstance, dayIndex: number, startMinutes: number) => void;
  onResize: (instance: EventInstance, endMinutes: number) => void;
}

export function useTimeGridDrag(options: TimeGridDragOptions) {
  const [preview, setPreviewState] = React.useState<DragPreview | null>(null);
  const previewRef = React.useRef<DragPreview | null>(null);
  const session = React.useRef<Session | null>(null);
  const suppressClick = React.useRef(false);
  const latest = React.useRef(options);
  latest.current = options;

  const setPreview = React.useCallback((next: DragPreview | null) => {
    previewRef.current = next;
    setPreviewState(next);
  }, []);

  const geometry = React.useCallback((clientX: number, clientY: number) => {
    const element = latest.current.containerRef.current;
    if (!element) return null;
    const rect = element.getBoundingClientRect();
    return {
      minutes: slotMath.clampMinutes(slotMath.pxToMinutes(clientY - rect.top)),
      dayIndex: columnFromX(clientX - rect.left, rect.width, latest.current.days.length),
    };
  }, []);

  React.useEffect(() => {
    const onPointerMove = (event: PointerEvent) => {
      const current = session.current;
      const point = geometry(event.clientX, event.clientY);
      if (!current || !point) return;
      if (
        !current.active &&
        Math.abs(event.clientX - current.originX) < MOVE_THRESHOLD &&
        Math.abs(event.clientY - current.originY) < MOVE_THRESHOLD
      ) {
        return;
      }
      current.active = true;
      if (current.kind === "create") {
        const range = dragCreateRange(current.anchorMinutes, point.minutes);
        setPreview({ kind: "create", dayIndex: current.dayIndex, instanceKey: null, ...range });
        return;
      }
      if (current.kind === "move") {
        const range = dragMoveRange(point.minutes - current.grabOffset, current.durationMinutes, 0);
        setPreview({
          kind: "move",
          dayIndex: point.dayIndex,
          instanceKey: current.instance?.key ?? null,
          ...range,
        });
        return;
      }
      setPreview({
        kind: "resize",
        dayIndex: current.dayIndex,
        instanceKey: current.instance?.key ?? null,
        startMinutes: current.startMinutes,
        endMinutes: dragResizeEnd(current.startMinutes, point.minutes),
      });
    };

    const onPointerUp = () => {
      const current = session.current;
      session.current = null;
      const snapshot = previewRef.current;
      setPreview(null);
      if (!current) return;
      if (!current.active || !snapshot) return;
      suppressClick.current = true;
      window.addEventListener(
        "click",
        () => {
          suppressClick.current = false;
        },
        { once: true },
      );
      if (current.kind === "create") {
        latest.current.onCreate(snapshot.dayIndex, snapshot.startMinutes, snapshot.endMinutes);
      } else if (current.kind === "move" && current.instance) {
        latest.current.onMove(current.instance, snapshot.dayIndex, snapshot.startMinutes);
      } else if (current.kind === "resize" && current.instance) {
        latest.current.onResize(current.instance, snapshot.endMinutes);
      }
    };

    window.addEventListener("pointermove", onPointerMove);
    window.addEventListener("pointerup", onPointerUp);
    return () => {
      window.removeEventListener("pointermove", onPointerMove);
      window.removeEventListener("pointerup", onPointerUp);
    };
  }, [geometry, setPreview]);

  const begin = (kind: Session["kind"], event: React.PointerEvent, instance: EventInstance | null) => {
    if (!options.enabled || event.button !== 0) return;
    const point = geometry(event.clientX, event.clientY);
    if (!point) return;
    const day = options.days[point.dayIndex];
    const clipped = instance && day ? clipToDay(instance.occurrence, day) : null;
    const startMinutes = clipped?.startMinutes ?? point.minutes;
    const endMinutes = clipped?.endMinutes ?? point.minutes;
    session.current = {
      kind,
      instance,
      dayIndex: point.dayIndex,
      anchorMinutes: point.minutes,
      startMinutes,
      durationMinutes: Math.max(endMinutes - startMinutes, 15),
      grabOffset: point.minutes - startMinutes,
      originX: event.clientX,
      originY: event.clientY,
      active: false,
    };
  };

  return {
    preview,
    isDragging: preview !== null,
    consumedClick: () => suppressClick.current,
    onBackgroundPointerDown: (event: React.PointerEvent) => begin("create", event, null),
    onBlockPointerDown: (event: React.PointerEvent, instance: EventInstance) =>
      begin("move", event, instance),
    onResizePointerDown: (event: React.PointerEvent, instance: EventInstance) =>
      begin("resize", event, instance),
  };
}
