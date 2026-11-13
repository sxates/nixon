'use client';

/**
 * Pointer-driven list reordering (specs/0088). HTML5 drag-and-drop is deliberately not
 * used: in the Tauri window any native drag session also reaches the window's file-drop
 * handler, which raised the "Drop audio file to import" overlay over the list. Pointer
 * events never start a native drag, so nothing outside the list sees them.
 *
 * Rows carry `data-reorder-id={id}`; the grip spreads `gripProps(id)`. While dragging,
 * the row under the pointer is the drop target; release moves the dragged id there.
 * Keyboard: ArrowUp / ArrowDown on a focused grip moves that row one place.
 */

import { useCallback, useRef, useState, type KeyboardEvent, type PointerEvent } from 'react';
import { reorderIds } from '@/lib/action-items';

export const REORDER_ATTR = 'data-reorder-id';

function rowIdAt(x: number, y: number): string | null {
  if (typeof document === 'undefined' || !document.elementFromPoint) return null;
  const el = document.elementFromPoint(x, y) as HTMLElement | null;
  return el?.closest<HTMLElement>(`[${REORDER_ATTR}]`)?.getAttribute(REORDER_ATTR) ?? null;
}

export function useDragReorder(ids: readonly string[], onReorder: (next: string[]) => void) {
  const [draggingId, setDraggingId] = useState<string | null>(null);
  const [overId, setOverId] = useState<string | null>(null);
  const overRef = useRef<string | null>(null);

  const reset = useCallback(() => {
    setDraggingId(null);
    setOverId(null);
    overRef.current = null;
  }, []);

  const moveTo = useCallback(
    (dragged: string, target: string | null) => {
      if (!target || target === dragged) return;
      onReorder(reorderIds(ids, dragged, target));
    },
    [ids, onReorder],
  );

  const gripProps = (id: string) => ({
    onPointerDown: (e: PointerEvent<HTMLElement>) => {
      if (e.button > 0) return; // primary button only (jsdom leaves it undefined)
      e.preventDefault();
      e.currentTarget.setPointerCapture?.(e.pointerId);
      overRef.current = id;
      setDraggingId(id);
      setOverId(id);
    },
    onPointerMove: (e: PointerEvent<HTMLElement>) => {
      if (draggingId !== id) return;
      const over = rowIdAt(e.clientX, e.clientY);
      if (over && over !== overRef.current) {
        overRef.current = over;
        setOverId(over);
      }
    },
    onPointerUp: (e: PointerEvent<HTMLElement>) => {
      if (draggingId !== id) return;
      e.currentTarget.releasePointerCapture?.(e.pointerId);
      moveTo(id, rowIdAt(e.clientX, e.clientY) ?? overRef.current);
      reset();
    },
    onPointerCancel: reset,
    onKeyDown: (e: KeyboardEvent<HTMLElement>) => {
      if (e.key !== 'ArrowUp' && e.key !== 'ArrowDown') return;
      e.preventDefault();
      const i = ids.indexOf(id);
      moveTo(id, ids[e.key === 'ArrowUp' ? i - 1 : i + 1] ?? null);
    },
  });

  return { draggingId, overId, gripProps };
}
