# DB Free — UI agent notes

The webview is **shadcn/ui**: Radix primitives plus `class-variance-authority`,
vendored as editable source under `src/components/ui`. There is no component
package to consult — the source in this repo is the API.

## Where things are

| Concern | File |
|---|---|
| Primitives (button, dialog, select, dropdown-menu, popover, tooltip, tabs, command, calendar…) | `src/components/ui/*.tsx` |
| Chat primitives (AI Elements: conversation, message, prompt-input, suggestion) | `src/components/ui/{conversation,message,prompt-input,suggestion}.tsx` |
| Typed form wrappers (`Field`, `AppSelect`, `Toggle`, `Check`, `Segmented`, `DateTimeField`, `NumberInput`) | `src/components/global/Field.tsx` |
| Design tokens, glass materials, density | `src/styles/globals.css` |
| Icons (Hugeicons, one registry) | `src/lib/icons.tsx` |
| shadcn CLI config | `components.json` |

## House rules

- **Icons:** Hugeicons only, via `Icon name="…"`. Components added from the
  shadcn registry arrive with lucide imports — swap them before committing.
- **Colour:** only tokens registered in `globals.css`. `scripts/guardrail.py`
  fails the build on a palette class (`bg-zinc-800`) or a hex value in a
  `className`.
- **Types:** no `any`, no `unknown` outside `src/lib/ipc.ts`, and no type
  assertions at all — `@typescript-eslint/consistent-type-assertions` is set to
  `never`. Upstream shadcn source sometimes uses `as`; rewrite it.
- **Overlays:** Radix needs a real trigger element. Use `asChild` so the app's
  own `Button` stays the trigger rather than nesting a button in a button.
- **Scrolling:** use `ScrollArea` from `src/components/ui/scroll-area.tsx`. It
  is native overflow with an edge mask, not the Radix ScrollArea — the data
  grid virtualizes against the real scrolling element, and the `min-h-0 flex-1`
  panels rely on intrinsic sizing a Radix viewport collapses.
- **Toasts:** Sonner. Fire them through the workspace store (`showInfo`,
  `showError`) so they stay consistent.

## Adding a component

```
npx shadcn@latest add <name>     # lands in src/components/ui
```

Then: replace lucide icons with `Icon`, replace any raw colour with a token,
remove assertions, and add the `// SOT:` line the guardrail requires.
