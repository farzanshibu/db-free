// SOT: connection-string-box, paste-connection-string-in-form, engine-mismatch-check
import { useState, type ClipboardEvent } from "react";
import { connectionStringMismatch, engineMeta, parseConnectionStringDetailed, type ParsedConnection } from "@/lib/engines";
import type { Engine } from "@/lib/bindings";
import { Icon } from "@/lib/icons";
import { Alert, AlertContent, AlertDescription, AlertIndicator } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { describeParsed } from "./formHelpers";

interface ConnectionStringBoxProps {
  /// The engine the form is currently set to.
  engine: Engine;
  /// Fill the form from the string; `force` switches the form to the string's engine.
  onApply: (parsed: ParsedConnection, force: boolean) => void;
}

// WHAT:  "Paste a connection string" inside the connection form: fills the fields,
//        and says so when the string belongs to a different engine.
// WHY:   People have the URL (from a dashboard, a .env file) and not the six
//        values it holds. The picker already did this on its first screen; the
//        form needs it too, for editing and for starting from a chosen engine.
// HOW:   A paste that parses and fits the form applies at once and the box
//        empties, so a password in the pasted URL does not sit in a visible
//        field. Typed text is previewed, and applies on Enter or the button.
//        A string for another engine is not applied until the user agrees to
//        switch. Query parameters the form has no field for are kept as options.
// WHERE: src/lib/engines.ts (parseConnectionStringDetailed), formHelpers.ts (mergeParsed)
export function ConnectionStringBox({ engine, onApply }: ConnectionStringBoxProps) {
  const [text, setText] = useState("");
  const [note, setNote] = useState<string | null>(null);
  const parsed = parseConnectionStringDetailed(text);
  const mismatch = parsed ? connectionStringMismatch(engine, parsed.engine) : null;

  const apply = (result: ParsedConnection, force: boolean) => {
    onApply(result, force);
    setText("");
    setNote(result.notes.length > 0 ? result.notes.join(" ") : `Filled in from the ${engineMeta(result.engine).label} connection string.`);
  };

  const paste = (event: ClipboardEvent<HTMLInputElement>) => {
    const pasted = event.clipboardData.getData("text");
    const result = parseConnectionStringDetailed(pasted);
    if (result && connectionStringMismatch(engine, result.engine) === null) {
      event.preventDefault();
      apply(result, false);
    }
  };

  return (
    <div className="flex flex-col gap-1.5">
      <Label htmlFor="connection-string">Connection string</Label>
      <div className="flex items-center gap-2">
        <Input
          id="connection-string"
          value={text}
          autoComplete="off"
          spellCheck={false}
          placeholder="Paste postgres://user:password@host:5432/database to fill the fields"
          className="font-mono text-xs"
          onChange={(event) => {
            setText(event.target.value);
            setNote(null);
          }}
          onPaste={paste}
          onKeyDown={(event) => {
            if (event.key === "Enter" && parsed && mismatch === null) apply(parsed, false);
          }}
        />
        {parsed && mismatch === null ? (
          <Button variant="secondary" size="sm" onClick={() => { apply(parsed, false); }}>
            Fill fields
          </Button>
        ) : null}
      </div>
      {parsed && mismatch === null ? (
        <p className="flex items-center gap-1.5 text-xs text-success">
          <Icon name="check" size={12} />
          {describeParsed(parsed, engineMeta(parsed.engine).label)}
        </p>
      ) : null}
      {text.trim().length > 0 && parsed === null ? <p className="text-xs text-danger">Not a connection string this app knows. Use scheme://user:password@host:port/database.</p> : null}
      {parsed && mismatch !== null ? (
        <Alert variant="warning">
          <AlertIndicator />
          <AlertContent className="gap-2">
            <AlertDescription>{mismatch}</AlertDescription>
            <div>
              <Button variant="secondary" size="sm" onClick={() => { apply(parsed, true); }}>
                Switch to {engineMeta(parsed.engine).label} and fill
              </Button>
            </div>
          </AlertContent>
        </Alert>
      ) : null}
      {note !== null ? <p className="selectable text-xs text-muted">{note}</p> : null}
    </div>
  );
}
