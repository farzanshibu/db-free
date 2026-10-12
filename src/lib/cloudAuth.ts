// SOT: cloud-auth-methods, auth-method-matrix, aws-regions, auth-defaults, auth-secret-fields
import type { AuthSecrets, AwsAccess, AwsCredentialSource, ConnectionAuth, ConnectionInput, Engine, Family, SslMode } from "./bindings";
import { ENGINE_FACTS } from "./bindings/EngineFacts.gen";

// WHAT:  The sign-in methods a connection can use, which engines offer them,
//        and the blank settings each starts from.
// WHY:   Rust owns the rules (`ConnectionAuth::allowed_for`, `validate`); this
//        file only decides what the form offers and keeps the pure logic out of
//        the component. The matrix below restates `allowed_for` for the picker,
//        keyed by the Rust-generated adapter family, so an engine moved to
//        another family follows it here without an edit.
// WHERE: src-tauri/src/model/auth.rs, src/features/connections/AuthSection.tsx

export type AuthMethod = ConnectionAuth["method"];

interface AuthMethodMeta {
  label: string;
  /// One line under the picker once the method is chosen.
  hint: string;
}

export const AUTH_METHODS = {
  password: { label: "Password", hint: "The username and password below, stored sealed with AES-256-GCM." },
  aws_rds_iam: {
    label: "AWS RDS IAM",
    hint: "A 15-minute token is signed locally for rds-db:connect and used as the password. Needs the database user to be enabled for IAM authentication.",
  },
  azure_ad: {
    label: "Azure AD (Microsoft Entra ID)",
    hint: "An access token for the database is requested from Microsoft Entra ID and refreshed before it expires.",
  },
  gcp_cloud_sql_iam: {
    label: "Google Cloud SQL IAM",
    hint: "A Google OAuth token is used as the password. The database user is the IAM account (a service account without .gserviceaccount.com).",
  },
  mongo_atlas_aws: { label: "Atlas AWS IAM (MONGODB-AWS)", hint: "Authenticates to MongoDB Atlas with an AWS IAM identity. No username or password." },
  mongo_atlas_oidc: { label: "Atlas workload identity (MONGODB-OIDC)", hint: "Authenticates to MongoDB Atlas with an Azure, Google Cloud or Kubernetes workload identity." },
  aws_credentials: { label: "AWS profile / environment", hint: "Credentials come from an AWS profile or the default credential chain instead of typed keys." },
} as const satisfies Record<AuthMethod, AuthMethodMeta>;

const SQL_CLOUD: readonly AuthMethod[] = ["password", "aws_rds_iam", "azure_ad", "gcp_cloud_sql_iam"];

// WHAT:  Methods per adapter family. Families not listed sign in with a password.
const BY_FAMILY: Partial<Record<Family, readonly AuthMethod[]>> = {
  postgres: SQL_CLOUD,
  mysql: SQL_CLOUD,
  // NTLM is not offered: the SQL Server driver speaks Windows authentication only on Windows.
  mssql: ["password", "azure_ad"],
  mongodb: ["password", "mongo_atlas_aws", "mongo_atlas_oidc"],
  dynamodb: ["password", "aws_credentials"],
};

export function authMethodsFor(engine: Engine): readonly AuthMethod[] {
  return BY_FAMILY[ENGINE_FACTS[engine].family] ?? ["password"];
}

/// Whether the engine offers any method besides the password.
export function hasAuthChoice(engine: Engine): boolean {
  return authMethodsFor(engine).length > 1;
}

export function emptyAuthSecrets(): AuthSecrets {
  return {
    awsSecretAccessKey: null,
    awsSessionToken: null,
    azureClientSecret: null,
    azurePassword: null,
    azureAccessToken: null,
    gcpServiceAccountJson: null,
  };
}

export function blankAwsAccess(): AwsAccess {
  return { source: { source: "default_chain" }, roleArn: null, roleSessionName: null, externalId: null };
}

// WHAT:  The settings a method starts with when it is picked.
export function defaultAuth(method: AuthMethod): ConnectionAuth {
  switch (method) {
    case "password":
      return { method: "password" };
    case "aws_rds_iam":
      return { method: "aws_rds_iam", region: null, credentials: blankAwsAccess(), caBundle: null };
    case "azure_ad":
      return { method: "azure_ad", mode: "default", tenantId: null, clientId: null, username: null, scope: null };
    case "gcp_cloud_sql_iam":
      return { method: "gcp_cloud_sql_iam", mode: "adc", impersonateServiceAccount: null };
    case "mongo_atlas_aws":
      return { method: "mongo_atlas_aws", credentials: blankAwsAccess() };
    case "mongo_atlas_oidc":
      return { method: "mongo_atlas_oidc", provider: "azure", tokenResource: null };
    case "aws_credentials":
      return { method: "aws_credentials", credentials: blankAwsAccess() };
  }
}

// WHAT:  The TLS mode a method needs to be useful: the token must not travel in
//        the clear, and RDS ships the CA to verify against. Returns the current
//        mode when it is already strict enough.
export function sslForMethod(method: AuthMethod, current: SslMode): SslMode {
  if (current !== "disable" && current !== "prefer") return current;
  switch (method) {
    case "aws_rds_iam":
      return "verify_ca";
    case "azure_ad":
    case "gcp_cloud_sql_iam":
    case "mongo_atlas_aws":
    case "mongo_atlas_oidc":
      return "require";
    case "password":
    case "aws_credentials":
      return current;
  }
}

/// The form shows the password field only for the password method.
export function authUsesPassword(input: ConnectionInput): boolean {
  return input.auth.method === "password";
}

// WHAT:  Whether the form asks for a database user. A token is issued *for* a
//        user on PostgreSQL / MySQL; SQL Server and MongoDB Atlas identify the
//        caller by the token itself.
export function authShowsUser(input: ConnectionInput): boolean {
  switch (input.auth.method) {
    case "password":
    case "aws_rds_iam":
    case "gcp_cloud_sql_iam":
      return true;
    case "azure_ad":
      return ENGINE_FACTS[input.engine].family !== "mssql";
    case "mongo_atlas_aws":
    case "mongo_atlas_oidc":
    case "aws_credentials":
      return false;
  }
}

export type AwsSourceKind = AwsCredentialSource["source"];

// WHAT:  A credential source of another kind, keeping what was typed when the
//        kind did not change.
export function sourceOfKind(kind: AwsSourceKind, previous: AwsCredentialSource): AwsCredentialSource {
  switch (kind) {
    case "default_chain":
      return { source: "default_chain" };
    case "profile":
      return { source: "profile", name: previous.source === "profile" ? previous.name : "" };
    case "access_keys":
      return { source: "access_keys", accessKeyId: previous.source === "access_keys" ? previous.accessKeyId : "" };
  }
}

// WHAT:  A DynamoDB-style region dropdown's options. A value outside the list
//        (a region added after this release, "localhost" for DynamoDB Local) is
//        kept as its own entry so the dropdown never silently changes it.
export function regionOptions(current: string): readonly { value: string; label: string }[] {
  const known = AWS_REGIONS.map((r) => ({ value: r.id, label: `${r.id} · ${r.name}` }));
  const trimmed = current.trim();
  return trimmed === "" || AWS_REGIONS.some((r) => r.id === trimmed) ? known : [{ value: trimmed, label: `${trimmed} (custom)` }, ...known];
}

export const AWS_REGIONS: readonly { id: string; name: string }[] = [
  { id: "us-east-1", name: "N. Virginia" },
  { id: "us-east-2", name: "Ohio" },
  { id: "us-west-1", name: "N. California" },
  { id: "us-west-2", name: "Oregon" },
  { id: "af-south-1", name: "Cape Town" },
  { id: "ap-east-1", name: "Hong Kong" },
  { id: "ap-south-1", name: "Mumbai" },
  { id: "ap-south-2", name: "Hyderabad" },
  { id: "ap-southeast-1", name: "Singapore" },
  { id: "ap-southeast-2", name: "Sydney" },
  { id: "ap-southeast-3", name: "Jakarta" },
  { id: "ap-southeast-4", name: "Melbourne" },
  { id: "ap-northeast-1", name: "Tokyo" },
  { id: "ap-northeast-2", name: "Seoul" },
  { id: "ap-northeast-3", name: "Osaka" },
  { id: "ca-central-1", name: "Canada" },
  { id: "ca-west-1", name: "Calgary" },
  { id: "eu-central-1", name: "Frankfurt" },
  { id: "eu-central-2", name: "Zurich" },
  { id: "eu-west-1", name: "Ireland" },
  { id: "eu-west-2", name: "London" },
  { id: "eu-west-3", name: "Paris" },
  { id: "eu-north-1", name: "Stockholm" },
  { id: "eu-south-1", name: "Milan" },
  { id: "eu-south-2", name: "Spain" },
  { id: "il-central-1", name: "Tel Aviv" },
  { id: "me-central-1", name: "UAE" },
  { id: "me-south-1", name: "Bahrain" },
  { id: "sa-east-1", name: "São Paulo" },
  { id: "us-gov-east-1", name: "GovCloud East" },
  { id: "us-gov-west-1", name: "GovCloud West" },
  { id: "cn-north-1", name: "Beijing" },
  { id: "cn-northwest-1", name: "Ningxia" },
];
