// SOT: chat-history-popover, chat-switch, chat-rename, chat-delete
import { useState } from "react";
import type { AiChatSummary } from "@/lib/bindings";
import { Icon } from "@/lib/icons";
import { cn } from "@/lib/cn";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { relativeTime } from "./chatMessages";

// WHAT:  The list of this connection's saved chats: switch to one, rename it,
//        delete it, or start a new one.
// WHY:   A chat is kept until you delete it, so there has to be a way back to an
//        old one. Each row says what it was about (its title), when it last moved
//        and how long it ran, which is usually enough to find it again.
// HOW:   Rename edits in place (Enter saves, Escape cancels). Delete asks again
//        on the same button rather than opening a dialog: it is one click to
//        mean it, two to do it, and a click elsewhere takes it back.
// WHERE: src/features/chat/ChatTab.tsx, src-tauri/src/commands/ai.rs (ai_chat_*)

interface ChatHistoryProps {
  chats: readonly AiChatSummary[];
  activeId: string;
  /// A run is in flight: switching away would orphan its stream.
  busy: boolean;
  onSelect: (chatId: string) => void;
  onNew: () => void;
  onRename: (chatId: string, title: string) => Promise<void>;
  onDelete: (chatId: string) => Promise<void>;
}

export function ChatHistory({ chats, activeId, busy, onSelect, onNew, onRename, onDelete }: ChatHistoryProps) {
  const [open, setOpen] = useState(false);
  const [renaming, setRenaming] = useState<string | null>(null);
  const [draft, setDraft] = useState("");
  const [confirming, setConfirming] = useState<string | null>(null);

  const commitRename = (chatId: string) => {
    const title = draft.trim();
    setRenaming(null);
    if (title.length > 0) void onRename(chatId, title);
  };

  return (
    <Popover
      open={open}
      onOpenChange={(next) => {
        setOpen(next);
        if (!next) {
          setRenaming(null);
          setConfirming(null);
        }
      }}
    >
      <Tooltip>
        <TooltipTrigger asChild>
          <PopoverTrigger asChild>
            <Button variant="ghost" size="icon-sm" aria-label="Chat history" className="rounded-lg liquid-hover">
              <Icon name="history" size={14} />
            </Button>
          </PopoverTrigger>
        </TooltipTrigger>
        <TooltipContent>Chat history</TooltipContent>
      </Tooltip>
      <PopoverContent align="end" className="w-80 p-0">
        <div className="flex items-center justify-between gap-2 border-b border-border/40 px-2.5 py-1.5">
          <span className="text-xs font-semibold text-foreground">Chats</span>
          <Button
            size="sm"
            variant="soft"
            className="h-6 gap-1 rounded-lg px-2 text-[11px]"
            disabled={busy}
            onClick={() => {
              onNew();
              setOpen(false);
            }}
          >
            <Icon name="plus" size={11} />
            New chat
          </Button>
        </div>
        <ScrollArea className="max-h-80">
          {chats.length === 0 ? (
            <p className="p-3 text-center text-[11px] text-muted">No saved chats yet. Ask something and it will be kept here.</p>
          ) : (
            <ul className="p-1">
              {chats.map((chat) => {
                const active = chat.id === activeId;
                return (
                  <li key={chat.id}>
                    {renaming === chat.id ? (
                      <div className="flex items-center gap-1 p-1">
                        <Input
                          autoFocus
                          value={draft}
                          aria-label="Chat title"
                          className="h-6 text-xs"
                          onChange={(event) => { setDraft(event.target.value); }}
                          onKeyDown={(event) => {
                            if (event.key === "Enter") commitRename(chat.id);
                            if (event.key === "Escape") setRenaming(null);
                          }}
                        />
                        <Button size="icon-sm" variant="soft" aria-label="Save title" onClick={() => { commitRename(chat.id); }}>
                          <Icon name="check" size={12} />
                        </Button>
                      </div>
                    ) : (
                      <div
                        className={cn(
                          "group/chat flex items-center gap-1 rounded-lg pr-1",
                          active ? "bg-accent/12" : "hover:bg-surface-secondary/70",
                        )}
                      >
                        <button
                          type="button"
                          disabled={busy && !active}
                          onClick={() => {
                            onSelect(chat.id);
                            setOpen(false);
                          }}
                          className="min-w-0 flex-1 px-2 py-1 text-left disabled:opacity-50"
                        >
                          <p className={cn("truncate text-xs", active ? "font-semibold text-accent" : "text-foreground")}>{chat.title}</p>
                          <p className="text-[10px] text-muted">
                            {relativeTime(chat.updatedAt)} · {chat.messageCount} {chat.messageCount === 1 ? "message" : "messages"}
                          </p>
                        </button>
                        <div className="flex shrink-0 items-center opacity-0 transition-opacity group-focus-within/chat:opacity-100 group-hover/chat:opacity-100">
                          <Button
                            size="icon-sm"
                            variant="ghost"
                            aria-label={`Rename ${chat.title}`}
                            className="size-6 min-w-6"
                            onClick={() => {
                              setDraft(chat.title);
                              setRenaming(chat.id);
                              setConfirming(null);
                            }}
                          >
                            <Icon name="pencil" size={11} />
                          </Button>
                          {confirming === chat.id ? (
                            <Button
                              size="sm"
                              variant="danger-soft"
                              className="h-6 rounded px-1.5 text-[10.5px]"
                              disabled={busy && active}
                              onClick={() => {
                                setConfirming(null);
                                void onDelete(chat.id);
                              }}
                            >
                              Delete?
                            </Button>
                          ) : (
                            <Button
                              size="icon-sm"
                              variant="ghost"
                              aria-label={`Delete ${chat.title}`}
                              className="size-6 min-w-6"
                              onClick={() => { setConfirming(chat.id); }}
                            >
                              <Icon name="trash" size={11} />
                            </Button>
                          )}
                        </div>
                      </div>
                    )}
                  </li>
                );
              })}
            </ul>
          )}
        </ScrollArea>
      </PopoverContent>
    </Popover>
  );
}
