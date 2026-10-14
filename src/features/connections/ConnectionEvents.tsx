// SOT: connection-events, os-opened-database, database-file-drop, save-transient-prompt
import { useEffect, useState } from "react";
import { dispatchClaimedDrop, isDatabasePath, onConnectionOpened, onFilesDropped } from "@/lib/connectionFiles";
import { ipc, normalizeError } from "@/lib/ipc";
import { Icon } from "@/lib/icons";
import { useWorkspace } from "@/stores/workspace";
import { toast } from "@/components/ui/sonner";

// Transient connections this run has already opened on its own accord (a cold start,
// an OS event), so a re-render never opens the same file twice. A drop always opens.
const opened = new Set<string>();
// Those it has already offered "Save as connection" for.
const announced = new Set<string>();
// Only one mounted instance listens, however many places mount the component.
let owner: symbol | null = null;

// WHAT:  "Save as connection" for a file opened for one session.
async function keepConnection(connectionId: string, name: string) {
  const store = useWorkspace.getState();
  try {
    await ipc("save_transient_connection", { id: connectionId, name: null });
    await store.refreshConnections();
    store.showInfo(`Saved "${name}" as a connection.`);
  } catch (raw) {
    store.showError(normalizeError(raw));
  }
}

// WHAT:  A database file was dropped on the window: open it (a saved connection for that file, else a transient one).
async function openDropped(path: string) {
  try {
    const summary = await ipc("open_database_file", { path });
    opened.add(summary.id);
    await openConnection(summary.id, summary.name, summary.extra.transient);
  } catch (raw) {
    useWorkspace.getState().showError(normalizeError(raw));
  }
}

// WHAT:  Offers to keep a file that was opened for one session.
function announce(connectionId: string, name: string) {
  toast(`Opened ${name}`, {
    description: "It is removed when you quit, unless you save it as a connection.",
    action: {
      label: "Save as connection",
      onClick: () => {
        void keepConnection(connectionId, name);
      },
    },
  });
}

// WHAT:  Opens a connection a file produced: refresh the list, connect, and — the
//        first time this run sees a transient one — offer to keep it.
async function openConnection(connectionId: string, name: string, transient: boolean) {
  const store = useWorkspace.getState();
  await store.refreshConnections();
  store.selectConnection(connectionId);
  if (transient && !announced.has(connectionId)) {
    announced.add(connectionId);
    announce(connectionId, name);
  }
}

// WHAT:  The listeners that turn "a database file arrived" into an open connection:
//        the OS handing the app a file (double-click, Open with, a second launch)
//        and a file dropped on the window. Renders a drop hint while one is dragged over.
// WHY:   Opening `shop.sqlite` should be one gesture, not New Connection → SQLite →
//        Browse. The file becomes a *transient* connection, removed on quit unless
//        saved, so looking at a file leaves nothing behind.
// HOW:   Mounted by the connections page, and meant to be mounted once near the
//        app root (it renders nothing but the hint): only the first mounted
//        instance listens. A file already open as a transient connection when the
//        list loads (a cold start from a double-click, which beats the webview)
//        is opened too. A screen can claim drops for itself (`claimFileDrops`).
// WHERE: src-tauri/src/open_files.rs, src/lib/connectionFiles.ts
export function ConnectionEvents() {
  const connections = useWorkspace((s) => s.connections);
  const ready = useWorkspace((s) => s.ready);
  const [hovering, setHovering] = useState(false);

  useEffect(() => {
    if (owner !== null) return;
    const me = Symbol("connection-events");
    owner = me;
    const releases: (() => void)[] = [];
    let cancelled = false;
    const keep = (pending: Promise<() => void>) => {
      void pending.then((release) => {
        if (cancelled) release();
        else releases.push(release);
      });
    };
    keep(
      onConnectionOpened((event) => {
        opened.add(event.connectionId);
        void openConnection(event.connectionId, event.name, true);
      }),
    );
    keep(
      onFilesDropped(
        (paths) => {
          if (dispatchClaimedDrop(paths)) return;
          for (const path of paths.filter(isDatabasePath)) void openDropped(path);
        },
        (paths) => { setHovering(paths?.some(isDatabasePath) === true); },
      ),
    );
    return () => {
      cancelled = true;
      for (const release of releases) release();
      if (owner === me) owner = null;
    };
  }, []);

  // A transient connection that exists before this component listened (cold start).
  useEffect(() => {
    if (!ready) return;
    for (const c of connections) {
      if (c.extra.transient && !opened.has(c.id)) {
        opened.add(c.id);
        void openConnection(c.id, c.name, true);
      }
    }
  }, [connections, ready]);

  if (!hovering) return null;
  return (
    <div className="pointer-events-none fixed inset-0 z-[60] flex items-center justify-center bg-backdrop/60 backdrop-blur-sm">
      <div className="flex flex-col items-center gap-2 rounded-xl border-2 border-dashed border-accent bg-popover px-5 py-4 text-center shadow-xl">
        <Icon name="database" size={20} className="text-accent" />
        <p className="text-[13px] font-semibold text-foreground">Drop to open this database</p>
        <p className="text-xs text-muted">SQLite files (.db, .sqlite, .sqlite3) open straight away.</p>
      </div>
    </div>
  );
}
