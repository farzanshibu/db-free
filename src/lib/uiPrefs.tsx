// SOT: ui-prefs-context, date-picker-toggle, shared-ui-preferences
import { createContext, useContext, useMemo, type ReactNode } from "react";

// WHAT:  The few user preferences that generic form controls need to know about,
//        handed down by context instead of read from the workspace store.
// WHY:   `DateTimeField` (components/global/Field.tsx) is used by the grid, the
//        filter bar, dashboards and forms. It should honour Settings → Data Grid →
//        "Date picker" without importing the app's store, which would tie a
//        generic control to one screen's state.
// HOW:   App.tsx provides the values from the saved settings; with no provider
//        (a control rendered on its own) every preference keeps its default.
// WHERE: src/App.tsx (provider), src/components/global/Field.tsx (consumer)
interface UiPrefs {
  /// Show the calendar / clock button on date and time fields.
  datePickerEnabled: boolean;
}

const DEFAULT_PREFS: UiPrefs = { datePickerEnabled: true };

const UiPrefsContext = createContext<UiPrefs>(DEFAULT_PREFS);

export function UiPrefsProvider({ datePickerEnabled, children }: { datePickerEnabled: boolean; children: ReactNode }) {
  const value = useMemo(() => ({ datePickerEnabled }), [datePickerEnabled]);
  return <UiPrefsContext.Provider value={value}>{children}</UiPrefsContext.Provider>;
}

export function useDatePickerEnabled(): boolean {
  return useContext(UiPrefsContext).datePickerEnabled;
}
