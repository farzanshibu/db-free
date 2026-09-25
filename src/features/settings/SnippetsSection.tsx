// SOT: snippets-settings, user-snippets-ui
import { useState } from "react";
import type { Snippet } from "@/lib/bindings";
import { BUILTIN_SNIPPETS, SNIPPET_LANGUAGES, isSnippetLanguage, snippetPreview, type SnippetLanguage } from "@/lib/snippets";
import { AppSelect, Field } from "@/components/global/Field";
import { Icon } from "@/lib/icons";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";

interface Editing {
  /// Index in the user list, or null for a new snippet.
  index: number | null;
  draft: Snippet;
}

const BLANK: Snippet = { prefix: "", name: "", body: "", language: "sql" };

function languageLabel(language: string): string {
  return SNIPPET_LANGUAGES.find((l) => l.value === language)?.label ?? language;
}

// WHAT:  Settings → Editor → Snippets: add, edit and delete the user's own
//        snippets, with the built-in set listed underneath for reference.
// WHY:   Built-ins cover the common statements; a team's own boilerplate
//        (audit columns, a tenant filter) belongs to the user.
// HOW:   Edits the draft's `snippets`; Save on the settings dock persists it.
//        A user snippet with a built-in's prefix and language replaces it.
// WHERE: src/lib/snippets.ts (registry, body syntax), AppSettings.snippets
export function SnippetsSection({ snippets, onChange }: { snippets: readonly Snippet[]; onChange: (next: Snippet[]) => void }) {
  const [editing, setEditing] = useState<Editing | null>(null);

  const patchDraft = (partial: Partial<Snippet>) => setEditing((e) => (e ? { ...e, draft: { ...e.draft, ...partial } } : e));
  const prefix = editing?.draft.prefix.trim() ?? "";
  const clash = editing !== null && snippets.some((sn, i) => i !== editing.index && sn.prefix === prefix && sn.language === editing.draft.language);
  const valid = editing !== null && /^[A-Za-z0-9_.-]+$/.test(prefix) && editing.draft.body.trim().length > 0 && !clash;

  const commit = () => {
    if (editing === null || !valid) return;
    const next = { ...editing.draft, prefix, name: editing.draft.name.trim() };
    onChange(editing.index === null ? [...snippets, next] : snippets.map((sn, i) => (i === editing.index ? next : sn)));
    setEditing(null);
  };

  return (
    <>
      <div className="flex items-center gap-3">
        <h2 className="text-sm font-semibold text-foreground">Snippets</h2>
        <Button size="sm" variant="secondary" className="ml-auto" disabled={editing !== null} onClick={() => setEditing({ index: null, draft: BLANK })}>
          <Icon name="plus" size={12} />
          Add snippet
        </Button>
      </div>
      <p className="text-xs text-muted">
        Type a prefix in the query editor and press Tab to expand it; Tab then walks the <span className="font-mono">{"${field}"}</span> placeholders. Snippets also appear in the suggestion list.
      </p>
      {editing !== null ? (
        <div className="flex flex-col gap-3 rounded-xl border border-border/60 bg-surface p-4">
          <div className="grid grid-cols-3 gap-3">
            <Field label="Prefix" value={editing.draft.prefix} onChange={(v) => patchDraft({ prefix: v })} placeholder="audit" mono autoFocus />
            <Field label="Name" value={editing.draft.name} onChange={(v) => patchDraft({ name: v })} placeholder="Audit columns" optional />
            <AppSelect<SnippetLanguage>
              label="Language"
              value={isSnippetLanguage(editing.draft.language) ? editing.draft.language : "any"}
              options={SNIPPET_LANGUAGES}
              onChange={(v) => patchDraft({ language: v })}
            />
          </div>
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="snippet-body" className="text-xs text-muted">
              Body
            </Label>
            <Textarea id="snippet-body" value={editing.draft.body} onChange={(e) => patchDraft({ body: e.target.value })} placeholder={"SELECT ${columns} FROM ${table};"} className="min-h-24" />
          </div>
          {clash ? <p className="text-xs text-warning">Another snippet already uses this prefix for this language.</p> : null}
          <div className="flex justify-end gap-2">
            <Button size="sm" variant="ghost" onClick={() => setEditing(null)}>
              Cancel
            </Button>
            <Button size="sm" disabled={!valid} onClick={commit}>
              {editing.index === null ? "Add" : "Update"}
            </Button>
          </div>
        </div>
      ) : null}
      {snippets.length === 0 ? (
        <p className="rounded-md border border-dashed border-border px-3 py-3 text-center text-xs text-muted">No snippets of your own yet.</p>
      ) : (
        <ul className="divide-y divide-separator rounded-md border border-border bg-surface">
          {snippets.map((sn, i) => (
            <li key={`${sn.language}:${sn.prefix}:${i}`} className="flex items-center gap-3 px-3 py-2 text-[13px]">
              <span className="w-20 shrink-0 truncate font-mono text-accent">{sn.prefix}</span>
              <div className="min-w-0 flex-1">
                <p className="truncate text-foreground">{sn.name.length > 0 ? sn.name : snippetPreview(sn.body)}</p>
                <p className="truncate font-mono text-[11px] text-muted">{snippetPreview(sn.body)}</p>
              </div>
              <Badge size="sm" variant="soft" className="shrink-0 text-[9.5px]">
                {languageLabel(sn.language)}
              </Badge>
              <Button size="icon-sm" variant="ghost" aria-label={`Edit ${sn.prefix}`} disabled={editing !== null} onClick={() => setEditing({ index: i, draft: sn })}>
                <Icon name="pencil" size={12} />
              </Button>
              <Button size="icon-sm" variant="ghost" aria-label={`Delete ${sn.prefix}`} onClick={() => onChange(snippets.filter((_, j) => j !== i))}>
                <Icon name="trash" size={12} />
              </Button>
            </li>
          ))}
        </ul>
      )}
      <h3 className="text-[11px] font-semibold tracking-wider text-muted uppercase">Built-in</h3>
      <ul className="divide-y divide-separator rounded-md border border-border bg-surface">
        {BUILTIN_SNIPPETS.map((sn) => (
          <li key={`${sn.language}:${sn.prefix}`} className="flex items-center gap-3 px-3 py-1.5 text-[12.5px]">
            <span className="w-20 shrink-0 truncate font-mono text-accent">{sn.prefix}</span>
            <span className="min-w-0 flex-1 truncate font-mono text-[11px] text-muted">{snippetPreview(sn.body)}</span>
            <Badge size="sm" variant="soft" className="shrink-0 text-[9.5px]">
              {languageLabel(sn.language)}
            </Badge>
          </li>
        ))}
      </ul>
    </>
  );
}
