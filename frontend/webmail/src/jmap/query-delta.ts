export interface QueryChangesDelta {
  removed: string[];
  added: { id: string; index: number }[];
}

export function applyQueryChanges(ids: string[], delta: QueryChangesDelta): string[] {
  const removed = new Set([...delta.removed, ...delta.added.map((entry) => entry.id)]);
  const next = ids.filter((id) => !removed.has(id));
  const additions = [...delta.added].sort((a, b) => a.index - b.index);
  for (const { id, index } of additions) {
    next.splice(Math.min(index, next.length), 0, id);
  }
  return next;
}

export function mergePage(ids: string[], page: string[], position: number): string[] {
  const head = ids.slice(0, Math.min(position, ids.length));
  const pageSet = new Set(page);
  return [...head.filter((id) => !pageSet.has(id)), ...page];
}
