// SOT: chat-history-panel, chat-history-popover, chat-switch, chat-rename, chat-delete
import { useState } from "react";
import type { AiChatSummary } from "@/lib/bindings";
import { Icon } from "@/lib/icons";
import { cn } from "@/lib/cn";
import { IconButton } from "@/components/global/Button";
import { EmptyState } from "@/components/global/EmptyState";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import { relativeTime } from "./chatMessages";

// WHAT:  The list of this connection's saved chats: switch to one, rename it,
//        delete it, or start a new one.
// WHY:   A chat is kept until you delete it, so there has to be a way back to an
//        old one. Each row says what it was about (its title), when it last moved
//        and how long it ran, which is usually enough to find it again. It is a
//        resizable side panel (opened from the toolbar's history button) rather
//        than a popover, so the list can stay open beside the conversation on a
//        wide window.
// HOW:   Rename edits in place (Enter saves, Escape cancels). Delete asks again
//        on the same button rather than opening a dialog: it is one click to
//        mean it, two to do it, and a click elsewhere takes it back.
// WHERE: src/features/chat/ChatTab.tsx (SplitLayout host), src-tauri/src/commands/ai.rs (ai_chat_*)

interface ChatHistoryProps {
  chats: readonly AiChatSummary[];
  activeId: string;
  /// A run is in flight: switching away would orphan its stream.
  busy: boolean;
  onSelect: (chatId: string) => void;
  onNew: () => void;
  onRename: (chatId: string, title: string) => Promise<void>;
  onDelete: (chatId: string) => Promise<void>;
  onClose: () => void;
}

export function ChatHistory({ chats, activeId, busy, onSelect, onNew, onRename, onDelete, onClose }: ChatHistoryProps) {
  const [renaming, setRenaming] = useState<string | null>(null);
  const [draft, setDraft] = useState("");
  const [confirming, setConfirming] = useState<string | null>(null);

  const commitRename = (chatId: string) => {
    const title = draft.trim();
    setRenaming(null);
    if (title.length > 0) void onRename(chatId, title);
  };

  return (
    <div className="flex h-full min-h-0 flex-col border-r border-border/40 bg-surface/40">
      <div className="flex app-toolbar shrink-0 items-center justify-between gap-1 border-b border-border/40 pl-2.5 pr-1">
        <span className="text-xs font-semibold text-foreground">Chats</span>
        <div className="flex items-center gap-0.5">
          <IconButton icon="plus" label="New chat" disabled={busy} onClick={onNew} size={13} />
          <IconButton icon="x" label="Close chat history" onClick={onClose} size={13} />
        </div>
      </div>
      <ScrollArea className="min-h-0 flex-1">
        {chats.length === 0 ? (
          <EmptyState size="inline" icon="history" title="No saved chats" tip="Ask something and the chat is kept here until you delete it." />
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
    </div>
  );
}
