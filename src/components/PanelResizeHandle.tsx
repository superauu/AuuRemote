import { useCallback, useRef, useState } from "react";

/**
 * Draggable vertical separator between panels. Drag adjusts the adjacent
 * panel's width, double-click restores the default, and Arrow keys nudge by
 * 16px (Home resets) for keyboard users.
 */
export function PanelResizeHandle({ label, value, min, max, defaultValue, side, onChange }: {
  label: string;
  value: number;
  min: number;
  max: number;
  defaultValue: number;
  /** "left" = panel sits left of the handle (drag right to widen); "right" = the mirror image. */
  side: "left" | "right";
  onChange: (width: number) => void;
}) {
  const [dragging, setDragging] = useState(false);
  const start = useRef<{ x: number; width: number } | undefined>(undefined);
  // Dragging right widens a left panel and narrows a right one.
  const sign = side === "left" ? 1 : -1;

  const clamp = useCallback(
    (width: number) => Math.min(max, Math.max(min, Math.round(width))),
    [max, min],
  );

  const onPointerDown = useCallback((event: React.PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) return;
    event.preventDefault();
    event.currentTarget.setPointerCapture(event.pointerId);
    start.current = { x: event.clientX, width: value };
    setDragging(true);
  }, [value]);

  const onPointerMove = useCallback((event: React.PointerEvent<HTMLDivElement>) => {
    if (!dragging || !start.current) return;
    onChange(clamp(start.current.width + (event.clientX - start.current.x) * sign));
  }, [clamp, dragging, onChange, sign]);

  const onPointerEnd = useCallback((event: React.PointerEvent<HTMLDivElement>) => {
    if (!dragging) return;
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
    start.current = undefined;
    setDragging(false);
  }, [dragging]);

  const onKeyDown = useCallback((event: React.KeyboardEvent<HTMLDivElement>) => {
    const direction = event.key === "ArrowRight" ? 1 : event.key === "ArrowLeft" ? -1 : 0;
    if (direction !== 0) {
      event.preventDefault();
      onChange(clamp(value + direction * sign * 16));
    } else if (event.key === "Home") {
      event.preventDefault();
      onChange(defaultValue);
    }
  }, [clamp, defaultValue, onChange, sign, value]);

  return (
    <div
      className="resize-handle"
      role="separator"
      aria-orientation="vertical"
      aria-label={label}
      aria-valuenow={Math.round(value)}
      aria-valuemin={min}
      aria-valuemax={max}
      tabIndex={0}
      data-dragging={dragging || undefined}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerEnd}
      onPointerCancel={onPointerEnd}
      onDoubleClick={() => onChange(defaultValue)}
      onKeyDown={onKeyDown}
    />
  );
}
