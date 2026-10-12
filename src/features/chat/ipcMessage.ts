// SOT: chat-error-message, ipc-error-text
import { errorMessage, normalizeError } from "@/lib/ipc";

// WHAT:  Anything a failed call threw, as one line of text.
// WHY:   Every catch in the chat wants the same thing — the message of the typed
//        error — and `normalizeError` + `errorMessage` is the pair that produces it.
export function ipcMessage(raw: Parameters<typeof normalizeError>[0]): string {
  return errorMessage(normalizeError(raw));
}
