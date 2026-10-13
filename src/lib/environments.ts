// SOT: environment-registry, env-colours, read-only-default
import type { Environment } from "./bindings";
import { keysOf } from "./records";

// WHAT:  Badge colour tokens and defaults per environment.
// WHERE: src/styles/globals.css (--color-env-*), src-tauri/src/model/connection.rs
export interface EnvironmentMeta {
  label: string;
  /// Three-to-five letter tag for tight spaces (a tab's connection chip).
  short: string;
  dot: string;
  text: string;
  stripe: string;
  /// Translucent wash behind a tab or a header of a connection in this
  /// environment; empty where the environment needs no warning.
  tint: string;
  readOnlyDefault: boolean;
  /// Draw a coloured edge across the workspace (EnvironmentEdge) while connected.
  tintWorkspace: boolean;
}

export const ENVIRONMENTS = {
  none: { label: "None", short: "", dot: "bg-muted", text: "text-muted", stripe: "bg-border", tint: "", readOnlyDefault: false, tintWorkspace: false },
  local: { label: "Local", short: "LOCAL", dot: "bg-env-local", text: "text-env-local", stripe: "bg-env-local", tint: "", readOnlyDefault: false, tintWorkspace: false },
  staging: { label: "Staging", short: "STG", dot: "bg-env-staging", text: "text-env-staging", stripe: "bg-env-staging", tint: "bg-env-staging/10", readOnlyDefault: false, tintWorkspace: true },
  production: { label: "Production", short: "PROD", dot: "bg-env-production", text: "text-env-production", stripe: "bg-env-production", tint: "bg-env-production/15", readOnlyDefault: true, tintWorkspace: true },
} satisfies Record<Environment, EnvironmentMeta>;

export const ENVIRONMENT_ORDER: Environment[] = keysOf(ENVIRONMENTS);

export function environmentMeta(env: Environment): EnvironmentMeta {
  return ENVIRONMENTS[env];
}
