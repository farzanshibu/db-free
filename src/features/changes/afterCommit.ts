// SOT: after-commit-hooks, statement-after-commit
// WHAT:  Follow-up work a staged statement needs once it has actually run (close the
//        tabs of a dropped table, reload the catalog, move tag links).
// WHY:   A schema form used to do this right after its own script ran. Once the
//        statement is staged, the commit happens later, from Pending Changes, so the
//        form cannot do it itself. The hook is kept here (not on the StagedChange)
//        because a change is sent to Rust as data and a function cannot travel.
// HOW:   `stageStatements` registers a hook under each change id before the change
//        is staged or committed. `commitPending` calls `runAfterCommit` with the ids
//        that landed; each hook runs once and is dropped.
// WHERE: src/features/changes/stageStatement.ts, src/features/changes/commitPending.ts
const hooks = new Map<string, () => Promise<void>>();

export function registerAfterCommit(changeIds: readonly string[], hook: () => Promise<void>): void {
  for (const id of changeIds) hooks.set(id, hook);
}

export function forgetAfterCommit(changeIds: readonly string[]): void {
  for (const id of changeIds) hooks.delete(id);
}

export async function runAfterCommit(changeIds: readonly string[]): Promise<void> {
  const toRun = new Set<() => Promise<void>>();
  for (const id of changeIds) {
    const hook = hooks.get(id);
    if (hook === undefined) continue;
    hooks.delete(id);
    toRun.add(hook);
  }
  for (const hook of toRun) {
    try {
      await hook();
    } catch {
      // The statement already committed; a failed follow-up must not turn that into an error.
    }
  }
}
