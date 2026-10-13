// SOT: chat-model-picker, merged-provider-models, model-refresh
import { useMemo, useState } from "react";
import type { AiModel, AiProfile, ModelChoice } from "@/lib/bindings";
import { Icon } from "@/lib/icons";
import { cn } from "@/lib/cn";
import { Button } from "@/components/ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { ScrollArea } from "@/components/ui/scroll-area";
import { SearchInput } from "@/components/ui/input";
import { Spinner } from "@/components/ui/spinner";
import { canListModels, useAiModels } from "./useAiModels";

// WHAT:  The model chooser in the message box: every provider's models in one
//        list, with a refresh per provider and a way to type an id by hand.
// WHY:   Different questions want different models, and with several providers
//        configured the choice belongs next to the message, not in Settings.
//        Lists come from the provider, so a new model appears without an update
//        of the app; the typed id is for the one that has not been listed yet.
// HOW:   Picking a model sets it for this and later messages (remembered across
//        restarts). The list is filtered by what is typed in the search box;
//        when nothing matches, the typed text offers itself as a model id for
//        each provider.
// WHERE: src/features/chat/ChatTab.tsx, src/features/chat/useAiModels.ts

interface ModelPickerProps {
  profiles: readonly AiProfile[];
  value: ModelChoice | null;
  onChange: (choice: ModelChoice) => void;
  disabled: boolean;
}

function labelOf(models: readonly AiModel[] | undefined, id: string): string {
  return models?.find((model) => model.id === id)?.label ?? id;
}

export function ModelPicker({ profiles, value, onChange, disabled }: ModelPickerProps) {
  const [open, setOpen] = useState(false);
  const [search, setSearch] = useState("");
  const { lists, errors, loading, refresh } = useAiModels(profiles);

  const query = search.trim().toLowerCase();
  const sections = useMemo(
    () =>
      profiles
        .filter((profile) => canListModels(profile) || value?.profileId === profile.id)
        .map((profile) => {
          const models = lists[profile.id]?.models ?? [];
          const shown = query === "" ? models : models.filter((m) => m.id.toLowerCase().includes(query) || m.label.toLowerCase().includes(query));
          return { profile, shown };
        }),
    [lists, profiles, query, value?.profileId],
  );

  const current = value === null ? null : profiles.find((profile) => profile.id === value.profileId) ?? null;
  const currentLabel = value === null ? "Choose a model" : labelOf(lists[value.profileId]?.models, value.model);
  const noMatches = query !== "" && sections.every((section) => section.shown.length === 0);

  const pick = (choice: ModelChoice) => {
    onChange(choice);
    setOpen(false);
    setSearch("");
  };

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <Button
          size="sm"
          variant="ghost"
          disabled={disabled}
          aria-label="Choose the model for this message"
          className="h-6 max-w-56 gap-1.5 rounded-md px-2 text-[11px] text-muted hover:text-foreground"
        >
          <Icon name="brain" size={12} className="text-accent" />
          <span className="truncate">{currentLabel}</span>
          {current !== null && profiles.length > 1 ? <span className="shrink-0 text-muted/70">· {current.label}</span> : null}
          <Icon name="chevron-down" size={10} />
        </Button>
      </PopoverTrigger>
      <PopoverContent align="start" side="top" className="w-80 p-0">
        <div className="border-b border-border/40 p-2">
          <SearchInput value={search} onChange={setSearch} placeholder="Search or type a model id…" aria-label="Search models" />
        </div>
        <ScrollArea className="max-h-72">
          <div className="p-1">
            {sections.length === 0 ? (
              <p className="p-2.5 text-[11px] text-muted">Add a provider and key in Settings → AI to choose a model.</p>
            ) : null}
            {sections.map(({ profile, shown }) => {
              const busy = loading.has(profile.id);
              const failure = errors[profile.id];
              return (
                <div key={profile.id} className="mb-1">
                  <div className="flex items-center justify-between gap-2 px-2 py-1">
                    <span className="truncate text-[10px] font-semibold tracking-wider text-muted uppercase">{profile.label}</span>
                    <Button
                      size="sm"
                      variant="ghost"
                      className="h-5 gap-1 rounded px-1 text-[10px] text-muted hover:text-foreground"
                      disabled={busy || !canListModels(profile)}
                      aria-label={`Refresh ${profile.label} models`}
                      onClick={() => { refresh(profile); }}
                    >
                      {busy ? <Spinner size="sm" /> : <Icon name="refresh" size={10} />}
                      Refresh
                    </Button>
                  </div>
                  {failure !== undefined ? <p className="px-2 pb-1 text-[10.5px] text-danger">{failure}</p> : null}
                  {shown.map((model) => {
                    const selected = value?.profileId === profile.id && value.model === model.id;
                    return (
                      <button
                        key={model.id}
                        type="button"
                        onClick={() => { pick({ profileId: profile.id, model: model.id }); }}
                        className={cn(
                          "flex w-full items-center gap-2 rounded-md px-2 py-1 text-left text-xs liquid-hover",
                          selected ? "bg-accent/12 text-accent" : "text-foreground hover:bg-surface-secondary",
                        )}
                      >
                        <span className="min-w-0 flex-1 truncate">{model.label}</span>
                        {model.label !== model.id ? <span className="max-w-[40%] shrink-0 truncate font-mono text-[10px] text-muted">{model.id}</span> : null}
                        {selected ? <Icon name="check" size={11} /> : null}
                      </button>
                    );
                  })}
                  {shown.length === 0 && failure === undefined && !busy && query === "" ? (
                    <p className="px-2 pb-1 text-[10.5px] text-muted">No models listed. Type an id above to use one anyway.</p>
                  ) : null}
                </div>
              );
            })}
            {noMatches
              ? profiles.map((profile) => (
                  <button
                    key={`custom-${profile.id}`}
                    type="button"
                    onClick={() => { pick({ profileId: profile.id, model: search.trim() }); }}
                    className="flex w-full items-center gap-2 rounded-md px-2 py-1 text-left text-xs text-foreground liquid-hover hover:bg-surface-secondary"
                  >
                    <Icon name="plus" size={11} className="text-accent" />
                    <span className="min-w-0 flex-1 truncate">
                      Use <span className="font-mono">{search.trim()}</span> with {profile.label}
                    </span>
                  </button>
                ))
              : null}
          </div>
        </ScrollArea>
      </PopoverContent>
    </Popover>
  );
}
