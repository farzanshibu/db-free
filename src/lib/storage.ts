// SOT: local-storage-helpers, safe-local-storage, stored-preference

// WHAT:  localStorage reads and writes that never throw.
// WHY:   Blocked site data makes the accessor throw; every caller here stores a
//        per-viewer convenience (a width, a folded panel), where a missing value
//        just means "use the default".
export function readStored(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

export function writeStored(key: string, value: string): void {
  try {
    localStorage.setItem(key, value);
  } catch {
    // ignore: the preference simply is not remembered
  }
}
