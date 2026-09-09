// SOT: toaster, toast-region, notifications-ui
import { Toaster as Sonner, toast } from "sonner";

// WHAT:  The shadcn/ui Toaster (Sonner), themed as a glass pill.
// WHY:   Toasts are fired from the workspace store — outside React — so the
//        region has to be an imperative singleton rather than context.
// HOW:   Mounted once in App.tsx; call `toast.success` / `toast.error` from
//        anywhere (the store's showInfo / showError wrap it).
// WHERE: https://ui.shadcn.com/docs/components/sonner
export function Toaster() {
  return (
    <Sonner
      theme="dark"
      position="bottom-right"
      visibleToasts={3}
      toastOptions={{
        classNames: {
          toast: "glass-modal !rounded-xl !border-0 !text-[13px] !text-foreground",
          description: "!text-muted",
          actionButton: "!bg-accent !text-accent-foreground",
          cancelButton: "!bg-surface-secondary !text-foreground",
          error: "!text-danger",
          success: "!text-success",
        },
      }}
      style={{ width: "380px" }}
    />
  );
}

export { toast };
