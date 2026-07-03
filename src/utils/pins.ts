const STORAGE_KEY = 'pinned_items'

export function getPinnedIds(): Set<string> {
  try {
    const stored = localStorage.getItem(STORAGE_KEY)
    return new Set<string>(stored ? JSON.parse(stored) : [])
  } catch {
    return new Set<string>()
  }
}

export function togglePin(id: string | number): boolean {
  const strId = String(id)
  const ids = getPinnedIds()
  if (ids.has(strId)) ids.delete(strId); else ids.add(strId)
  localStorage.setItem(STORAGE_KEY, JSON.stringify([...ids]))
  return ids.has(strId)
}

export function sortPinnedFirst<T extends { id: string | number }>(items: T[]): T[] {
  const pinned = getPinnedIds()

  // Use a single O(N) pass rather than O(N log N) .toSorted
  // This is highly preferred for binary sorting (pinned vs unpinned)
  const pinnedItems: T[] = []
  const unpinnedItems: T[] = []

  for (const item of items) {
    if (pinned.has(String(item.id))) {
      pinnedItems.push(item)
    } else {
      unpinnedItems.push(item)
    }
  }

  return [...pinnedItems, ...unpinnedItems]
}
