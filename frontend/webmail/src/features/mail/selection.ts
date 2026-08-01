export interface Selection {
  selected: Set<string>;
  anchor: string | null;
}

export const emptySelection: Selection = { selected: new Set(), anchor: null };

export interface ClickModifiers {
  toggle: boolean;
  range: boolean;
}

export function applySelectionClick(
  state: Selection,
  orderedIds: string[],
  id: string,
  modifiers: ClickModifiers,
): Selection {
  if (modifiers.range && state.anchor != null) {
    const from = orderedIds.indexOf(state.anchor);
    const to = orderedIds.indexOf(id);
    if (from === -1 || to === -1) return { selected: new Set([id]), anchor: id };
    const [start, end] = from <= to ? [from, to] : [to, from];
    const selected = new Set(state.selected);
    for (const rangeId of orderedIds.slice(start, end + 1)) selected.add(rangeId);
    return { selected, anchor: state.anchor };
  }
  if (modifiers.toggle) {
    const selected = new Set(state.selected);
    if (selected.has(id)) selected.delete(id);
    else selected.add(id);
    return { selected, anchor: id };
  }
  return { selected: new Set(), anchor: id };
}
