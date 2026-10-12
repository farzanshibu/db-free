// SOT: create-key-dialog, redis-key-creation, new-key-form
import { useState, type ReactElement } from "react";
import type { RedisKeyType } from "@/lib/bindings";
import { normalizeError } from "@/lib/ipc";
import { parseJson } from "@/lib/json";
import { redisLine } from "@/lib/redisCommands";
import { useWorkspace } from "@/stores/workspace";
import { AppSelect, Field, NumberInput } from "@/components/global/Field";
import { Button } from "@/components/ui/button";
import { Dialog, DialogBody, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle, DialogTrigger } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { Icon } from "@/lib/icons";
import { parseScore } from "./collectionCommands";
import { keyExists, runRedis } from "./keyCommands";
import { KEY_TYPE_LABEL, TTL_UNITS, ttlSeconds, type TtlUnit } from "./keyFormat";

// WHAT:  The "New key" form: a name, a type, the type's first contents and an
//        optional expiry. Refuses a name that is already taken.
// WHY:   Redis has no empty collections, so a key is created together with its
//        first element; and a plain SET over an existing name would silently
//        overwrite someone's data.
// HOW:   Existence is checked on the read lane (`key_info`) before anything is
//        written. The key and its expiry go out as command lines through
//        `execute_query`, so a read-only connection refuses them.
// WHERE: src/features/shell/KeyTree.tsx (the "New key" button)
export type NewKeyType = Exclude<RedisKeyType, "other">;

const TYPES: readonly NewKeyType[] = ["string", "hash", "list", "set", "zset", "json", "stream"];

interface Pair {
  a: string;
  b: string;
}

const EMPTY_PAIR: Pair = { a: "", b: "" };

interface CreateKeyDialogProps {
  connectionId: string;
  trigger: ReactElement;
  /// The key exists now. Fired after the dialog has closed.
  onCreated: (name: string, kind: NewKeyType) => void;
}

// WHAT:  The command lines that create the key, or the reason they cannot be built.
export function createCommands(
  name: string,
  kind: NewKeyType,
  input: { text: string; pairs: readonly Pair[]; ttl: number | null },
): { lines: string[] } | { problem: string } {
  const filled = input.pairs.filter((p) => p.a !== "" || p.b !== "");
  let line: string;
  switch (kind) {
    case "string":
      line = redisLine("SET", name, input.text);
      break;
    case "json": {
      const parsed = parseJson(input.text);
      if (parsed === undefined) return { problem: "The JSON document is not valid." };
      line = redisLine("JSON.SET", name, "$", JSON.stringify(parsed));
      break;
    }
    case "hash":
    case "stream": {
      if (filled.length === 0 || filled.some((p) => p.a === "")) return { problem: kind === "hash" ? "Give every field a name, and add at least one." : "Give every entry field a name, and add at least one." };
      const words = filled.flatMap((p) => [p.a, p.b]);
      line = kind === "hash" ? redisLine("HSET", name, ...words) : redisLine("XADD", name, "*", ...words);
      break;
    }
    case "list":
    case "set": {
      const items = filled.map((p) => p.a).filter((a) => a !== "");
      if (items.length === 0) return { problem: kind === "list" ? "Add at least one element." : "Add at least one member." };
      line = redisLine(kind === "list" ? "RPUSH" : "SADD", name, ...items);
      break;
    }
    case "zset": {
      if (filled.length === 0) return { problem: "Add at least one member." };
      const words: string[] = [];
      for (const pair of filled) {
        const score = parseScore(pair.a);
        if (score === null || pair.b === "") return { problem: "Every member needs a numeric score and a name." };
        words.push(score, pair.b);
      }
      line = redisLine("ZADD", name, ...words);
      break;
    }
  }
  const lines = [line];
  if (input.ttl !== null) lines.push(redisLine("EXPIRE", name, String(input.ttl)));
  return { lines };
}

export function CreateKeyDialog({ connectionId, trigger, onCreated }: CreateKeyDialogProps) {
  const showError = useWorkspace((s) => s.showError);
  const showInfo = useWorkspace((s) => s.showInfo);
  const readOnly = useWorkspace((s) => s.connections.find((c) => c.id === connectionId)?.readOnly ?? false);
  const [open, setOpen] = useState(false);
  const [name, setName] = useState("");
  const [kind, setKind] = useState<NewKeyType>("string");
  const [text, setText] = useState("");
  const [pairs, setPairs] = useState<readonly Pair[]>([EMPTY_PAIR]);
  const [amount, setAmount] = useState<number | null>(null);
  const [unit, setUnit] = useState<TtlUnit>("s");
  const [problem, setProblem] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const reset = () => {
    setName("");
    setText("");
    setPairs([EMPTY_PAIR]);
    setAmount(null);
    setProblem(null);
  };

  const submit = async () => {
    // The name is kept exactly as typed (a key may start or end with a space);
    // only an empty one is refused.
    if (name.trim() === "") {
      setProblem("Give the key a name.");
      return;
    }
    const trimmed = name;
    const ttl = amount === null ? null : ttlSeconds(amount, unit);
    if (amount !== null && ttl === null) {
      setProblem("The expiry must be a whole, positive amount of time.");
      return;
    }
    const built = createCommands(trimmed, kind, { text, pairs, ttl });
    if ("problem" in built) {
      setProblem(built.problem);
      return;
    }
    setBusy(true);
    setProblem(null);
    try {
      if (await keyExists(connectionId, trimmed)) {
        setProblem(`A key named "${trimmed}" already exists.`);
        return;
      }
      await runRedis(connectionId, built.lines);
      showInfo(`Created ${trimmed}.`);
      setOpen(false);
      reset();
      onCreated(trimmed, kind);
    } catch (raw) {
      showError(normalizeError(raw));
    } finally {
      setBusy(false);
    }
  };

  const setPair = (index: number, part: Partial<Pair>) => {
    setPairs((current) => current.map((p, i) => (i === index ? { ...p, ...part } : p)));
  };

  const columns = kind === "hash" || kind === "stream" ? (["Field", "Value"] as const) : kind === "zset" ? (["Score", "Member"] as const) : kind === "list" || kind === "set" ? ([kind === "list" ? "Element" : "Member"] as const) : null;

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        setOpen(next);
        if (!next) setProblem(null);
      }}
    >
      <DialogTrigger asChild>{trigger}</DialogTrigger>
      <DialogContent className="sm:max-w-[520px]">
        <DialogHeader>
          <DialogTitle>New key</DialogTitle>
          <DialogDescription>{readOnly ? "This connection is read-only, so a key cannot be created." : "The key is created with its first contents; Redis has no empty collections."}</DialogDescription>
        </DialogHeader>
        <DialogBody className="flex flex-col gap-2">
          <Field label="Key name" value={name} onChange={setName} mono autoFocus placeholder="app:users:42" />
          <div className="grid grid-cols-2 gap-2">
            <AppSelect label="Type" value={kind} options={TYPES.map((t) => ({ value: t, label: KEY_TYPE_LABEL[t] }))} onChange={setKind} />
            <div className="flex flex-col gap-1.5">
              <Label>Expires after (optional)</Label>
              <div className="grid grid-cols-[1fr_7rem] gap-2">
                <NumberInput ariaLabel="Expires after" value={amount} onChange={setAmount} integer />
                <AppSelect ariaLabel="Unit" value={unit} options={TTL_UNITS.map((u) => ({ value: u.value, label: u.label }))} onChange={setUnit} />
              </div>
            </div>
          </div>
          {kind === "string" || kind === "json" ? (
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="new-key-value">{kind === "json" ? "JSON document" : "Value"}</Label>
              <Textarea
                id="new-key-value"
                value={text}
                onChange={(event) => {
                  setText(event.target.value);
                }}
                spellCheck={false}
                rows={7}
                placeholder={kind === "json" ? '{ "name": "Ann" }' : ""}
                className="font-mono text-xs"
              />
            </div>
          ) : columns !== null ? (
            <div className="flex flex-col gap-1.5">
              <div className="grid items-center gap-2" style={{ gridTemplateColumns: `repeat(${columns.length}, minmax(0, 1fr)) 2rem` }}>
                {columns.map((c) => (
                  <Label key={c}>{c}</Label>
                ))}
                <span />
                {pairs.flatMap((pair, index) => [
                  <Input
                    key={`${index}a`}
                    aria-label={`${columns[0]} ${index + 1}`}
                    value={pair.a}
                    onChange={(event) => {
                      setPair(index, { a: event.target.value });
                    }}
                    spellCheck={false}
                    className="font-mono text-xs"
                  />,
                  ...(columns.length === 2
                    ? [
                        <Input
                          key={`${index}b`}
                          aria-label={`${columns[1]} ${index + 1}`}
                          value={pair.b}
                          onChange={(event) => {
                            setPair(index, { b: event.target.value });
                          }}
                          spellCheck={false}
                          className="font-mono text-xs"
                        />,
                      ]
                    : []),
                  <Button
                    key={`${index}x`}
                    variant="ghost"
                    size="icon-sm"
                    aria-label="Remove row"
                    disabled={pairs.length === 1}
                    onClick={() => {
                      setPairs((current) => current.filter((_, i) => i !== index));
                    }}
                  >
                    <Icon name="x" size={13} />
                  </Button>,
                ])}
              </div>
              <Button
                size="sm"
                variant="ghost"
                className="self-start"
                onClick={() => {
                  setPairs((current) => [...current, EMPTY_PAIR]);
                }}
              >
                <Icon name="plus" size={12} />
                Add row
              </Button>
            </div>
          ) : null}
          {problem !== null ? <p className="text-xs text-danger">{problem}</p> : null}
        </DialogBody>
        <DialogFooter>
          <Button
            variant="secondary"
            onClick={() => {
              setOpen(false);
            }}
          >
            Cancel
          </Button>
          <Button pending={busy} disabled={readOnly} onClick={() => void submit()}>
            Create key
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
