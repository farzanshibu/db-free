// SOT: shell-migrations, store-migration-steps

// WHAT:  Workbench shell: this feature's store migration steps, in order.
// HOW:   Append a new step to change the schema; never edit a released one.
// WHERE: src-tauri/src/store/migrations/mod.rs (applies every list)
pub const STEPS: &[&str] = &[
    // 1 — the compact UI: installs still on the old defaults (13 px UI font,
    //     "cozy" grid) move to the new ones. A value the user picked is kept.
    "UPDATE settings SET value = json_set(value, '$.uiFontSize', 12)
       WHERE key = 'app' AND json_valid(value) AND json_extract(value, '$.uiFontSize') = 13;
     UPDATE settings SET value = json_set(value, '$.gridDensity', 'compact')
       WHERE key = 'app' AND json_valid(value) AND json_extract(value, '$.gridDensity') = 'cozy';",
];

#[cfg(test)]
mod tests {
    use super::STEPS;
    use crate::store::Store;

    fn run_step_on(saved: &str) -> String {
        let store = Store::open_in_memory().unwrap();
        store.set_setting("app", saved).unwrap();
        store.conn().execute_batch(STEPS[0]).unwrap();
        store.get_setting("app").unwrap().unwrap_or_default()
    }

    #[test]
    fn old_defaults_move_to_compact() {
        let after: serde_json::Value = serde_json::from_str(&run_step_on(r#"{"uiFontSize":13,"gridDensity":"cozy","theme":"dark"}"#)).unwrap();
        assert_eq!(after["uiFontSize"], 12);
        assert_eq!(after["gridDensity"], "compact");
        assert_eq!(after["theme"], "dark");
    }

    #[test]
    fn chosen_values_are_kept() {
        let after: serde_json::Value = serde_json::from_str(&run_step_on(r#"{"uiFontSize":14,"gridDensity":"comfortable"}"#)).unwrap();
        assert_eq!(after["uiFontSize"], 14);
        assert_eq!(after["gridDensity"], "comfortable");
    }
}
