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
 *
 * Options (specs/0090): `attr` names the marker attribute, so two reorderable levels can
 * nest (To-do sections, each holding rows) without one reading the other's targets.
 * `onDropOutside` gets a release that lands anywhere but one of this list's own rows,
 * with the element under the pointer (or the last row hovered), so a caller can move the
 * item to another list. Without it such a drop changes nothing.
 */

import { useCallback, useRef, useState, type KeyboardEvent, type PointerEvent } from 'react';
import { reorderIds } from '@/lib/action-items';

export const REORDER_ATTR = 'data-reorder-id';

function elementAt(x: number, y: number): HTMLElement | null {
  if (typeof document === 'undefined' || !document.elementFromPoint) return null;
  return (document.elementFromPoint(x, y) as HTMLElement | null) ?? null;
}

const cssEscape = (v: string) =>
  typeof CSS !== 'undefined' && CSS.escape ? CSS.escape(v) : v.replace(/["\\]/g, '\\$&');

const idIn = (el: Element | null, attr: string) =>
  el?.closest<HTMLElement>(`[${attr}]`)?.getAttribute(attr) ?? null;

export interface DragReorderOptions {
  /** The marker attribute on reorderable items. Default `data-reorder-id`. */
  attr?: string;
  /**
   * A drop that isn't on one of this list's own items. Return true when handled; false
   * falls back to the last own item hovered (or nothing).
   */
  onDropOutside?: (draggedId: string, target: HTMLElement | null) => boolean;
}

export function useDragReorder(
  ids: readonly string[],
  onReorder: (next: string[]) => void,
  { attr = REORDER_ATTR, onDropOutside }: DragReorderOptions = {},
) {
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
      const over = idIn(elementAt(e.clientX, e.clientY), attr);
      if (over && over !== overRef.current) {
        overRef.current = over;
        setOverId(over);
      }
    },
    onPointerUp: (e: PointerEvent<HTMLElement>) => {
      if (draggingId !== id) return;
      e.currentTarget.releasePointerCapture?.(e.pointerId);
      const el = elementAt(e.clientX, e.clientY);
      const rowId = idIn(el, attr);
      const lastId = rowId ?? overRef.current;
      const fallback = lastId
        ? document.querySelector<HTMLElement>(`[${attr}="${cssEscape(lastId)}"]`)
        : null;
      if (rowId && ids.includes(rowId)) moveTo(id, rowId);
      else if (onDropOutside?.(id, el ?? fallback)) {
        // handled by the caller (moved to another list)
      } else if (!rowId && overRef.current && ids.includes(overRef.current)) {
        moveTo(id, overRef.current);
      }
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
