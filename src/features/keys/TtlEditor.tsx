// SOT: ttl-editor, redis-key-expiry, expiry-countdown
import { useEffect, useState } from "react";
import { AppSelect, NumberInput } from "@/components/global/Field";
import { Button } from "@/components/ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { Icon } from "@/lib/icons";
import { redisLine } from "@/lib/redisCommands";
import { normalizeError } from "@/lib/ipc";
import { useWorkspace } from "@/stores/workspace";
import { formatRemaining, TTL_UNITS, ttlSeconds, type TtlUnit } from "./keyFormat";
import { runRedis } from "./keyCommands";

// WHAT:  The key's expiry as a chip with a live countdown; its popover sets a new
//        expiry (a number and a unit) or removes it.
// WHY:   Presets only cover the common cases; a cache entry that should live 36
//        hours needs a free number. The countdown ticks between server reads so
//        the chip never looks frozen.
// HOW:   EXPIRE / PERSIST through `execute_query`, so the read-only lock applies
//        (the editor is disabled on a read-only connection as well).
// WHERE: src/features/keys/KeyTab.tsx
interface TtlEditorProps {
  connectionId: string;
  keyName: string;
  /// When the key expires (epoch milliseconds), or null for no expiry.
  expiresAt: number | null;
  readOnly: boolean;
  onChanged: () => void;
}

function useNow(active: boolean): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!active) return;
    const id = window.setInterval(() => {
      setNow(Date.now());
    }, 1000);
    return () => {
      window.clearInterval(id);
    };
  }, [active]);
  return now;
}

export function TtlEditor({ connectionId, keyName, expiresAt, readOnly, onChanged }: TtlEditorProps) {
  const showError = useWorkspace((s) => s.showError);
  const showInfo = useWorkspace((s) => s.showInfo);
  const [open, setOpen] = useState(false);
  const [amount, setAmount] = useState<number | null>(null);
  const [unit, setUnit] = useState<TtlUnit>("s");
  const [problem, setProblem] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const now = useNow(expiresAt !== null);

  const remaining = expiresAt === null ? null : Math.max(0, expiresAt - now);
  const label = remaining === null ? "No expiry" : remaining === 0 ? "Expired" : formatRemaining(remaining);

  const send = async (line: string, message: string) => {
    setBusy(true);
    try {
      await runRedis(connectionId, [line]);
      showInfo(message);
      setOpen(false);
      onChanged();
    } catch (raw) {
      showError(normalizeError(raw));
    } finally {
      setBusy(false);
    }
  };

  const apply = () => {
    const seconds = ttlSeconds(amount, unit);
    if (seconds === null) {
      setProblem("Enter a whole, positive amount of time.");
      return;
    }
    setProblem(null);
    void send(redisLine("EXPIRE", keyName, String(seconds)), `${keyName} now expires in ${seconds}s.`);
  };

  return (
    <Popover
      open={open}
      onOpenChange={(next) => {
        setOpen(next);
        if (next) setProblem(null);
      }}
    >
      <PopoverTrigger asChild>
        <Button size="xs" variant="tertiary" aria-label={`Expiry: ${label}`} className="font-mono">
          <Icon name="clock" size={12} />
          {label}
        </Button>
      </PopoverTrigger>
      <PopoverContent align="end" className="w-72 space-y-2 p-2.5">
        <div className="space-y-0.5">
          <p className="text-xs font-medium text-foreground">Time to live</p>
          <p className="text-xs text-muted">{expiresAt === null ? "This key never expires." : `Expires ${new Date(expiresAt).toLocaleString()}.`}</p>
        </div>
        <div className="grid grid-cols-[1fr_7rem] items-end gap-2">
          <NumberInput ariaLabel="Expire in" value={amount} onChange={setAmount} integer disabled={readOnly || busy} />
          <AppSelect ariaLabel="Unit" value={unit} options={TTL_UNITS.map((u) => ({ value: u.value, label: u.label }))} onChange={setUnit} size="sm" disabled={readOnly || busy} />
        </div>
        {problem !== null ? <p className="text-xs text-danger">{problem}</p> : null}
        <div className="flex justify-between gap-2">
          <Button
            size="sm"
            variant="secondary"
            disabled={readOnly || busy || expiresAt === null}
            onClick={() => {
              void send(redisLine("PERSIST", keyName), "Expiry removed.");
            }}
          >
            Remove TTL
          </Button>
          <Button size="sm" disabled={readOnly || amount === null} pending={busy} onClick={apply}>
            Apply
          </Button>
        </div>
      </PopoverContent>
    </Popover>
  );
}
