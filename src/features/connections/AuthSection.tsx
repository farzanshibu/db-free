// SOT: auth-section, cloud-sign-in-form, aws-credentials-form, auth-method-select
import { useState } from "react";
import type { AuthSecrets, AwsAccess, AzureAdMode, ConnectionAuth, ConnectionInput, ConnectionSummary, GcpAuthMode, OidcProvider } from "@/lib/bindings";
import { AUTH_METHODS, authMethodsFor, blankAwsAccess, defaultAuth, hasAuthChoice, regionOptions, sourceOfKind, sslForMethod, type AuthMethod, type AwsSourceKind } from "@/lib/cloudAuth";
import { engineMeta, fieldLabels } from "@/lib/engines";
import { IconButton } from "@/components/global/Button";
import { AppSelect, Field, type Option } from "@/components/global/Field";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { Icon } from "@/lib/icons";

// WHAT:  The "how does this connection sign in" part of the connection form: a
//        method picker for engines that offer more than a password, and the
//        fields of the chosen method.
// WHY:   Enterprise databases hand out short-lived tokens instead of passwords
//        (AWS RDS IAM, Azure AD, Google Cloud SQL IAM, MongoDB Atlas workload
//        identity). Which methods exist for an engine, and what each needs, is
//        decided in Rust (`model::auth`); this component only draws it.
// HOW:   Secrets (keys, client secrets, tokens, a service-account key) go to
//        `input.authSecrets`, write-only like the password: a stored one shows
//        "•••• (unchanged)" and a blank field keeps it. Everything else lives in
//        `input.auth`. The AWS-form engines (DynamoDB, QLDB, S3) get their whole
//        credentials block from here, because their region, key and token fields
//        belong together.
// WHERE: src/features/connections/ConnectionForm.tsx (mounts it),
//        src/lib/cloudAuth.ts (method matrix, defaults), src-tauri/src/model/auth.rs

interface AuthSectionProps {
  input: ConnectionInput;
  /// The saved connection being edited, for the "(unchanged)" hints; null when creating.
  editing: ConnectionSummary | null;
  patch: (partial: Partial<ConnectionInput>) => void;
}

type SecretKey = keyof AuthSecrets;

interface SecretProps {
  input: ConnectionInput;
  editing: ConnectionSummary | null;
  patch: (partial: Partial<ConnectionInput>) => void;
  reveal: boolean;
  onToggleReveal: () => void;
}

const nullable = (value: string): string | null => (value.trim() === "" ? null : value);

export function AuthSection({ input, editing, patch }: AuthSectionProps) {
  const [reveal, setReveal] = useState(false);
  const toggle = () => { setReveal((shown) => !shown); };
  const shared: SecretProps = { input, editing, patch, reveal, onToggleReveal: toggle };

  if (engineMeta(input.engine).form === "aws") return <AwsForm {...shared} />;
  if (!hasAuthChoice(input.engine)) return null;

  const auth = input.auth;
  const choose = (method: AuthMethod) => {
    if (method !== auth.method) patch({ auth: defaultAuth(method), sslMode: sslForMethod(method, input.sslMode) });
  };
  const setAuth = (next: ConnectionAuth) => { patch({ auth: next }); };
  const options: readonly Option<AuthMethod>[] = authMethodsFor(input.engine).map((method) => ({ value: method, label: AUTH_METHODS[method].label }));

  return (
    <div className="flex flex-col gap-3">
      <AppSelect label="Authentication" value={auth.method} options={options} onChange={choose} />
      {auth.method === "password" ? null : <p className="-mt-3 text-xs text-muted">{AUTH_METHODS[auth.method].hint}</p>}
      <MethodFields auth={auth} setAuth={setAuth} {...shared} />
    </div>
  );
}

// WHAT:  The fields of whichever method is chosen. The password method has none
//        here: its username and password stay in the form's own fields.
function MethodFields({ auth, setAuth, ...secrets }: SecretProps & { auth: ConnectionAuth; setAuth: (next: ConnectionAuth) => void }) {
  switch (auth.method) {
    case "password":
      return null;
    case "aws_rds_iam":
      return <RdsFields auth={auth} setAuth={setAuth} {...secrets} />;
    case "azure_ad":
      return <AzureFields auth={auth} setAuth={setAuth} {...secrets} />;
    case "gcp_cloud_sql_iam":
      return <GcpFields auth={auth} setAuth={setAuth} {...secrets} />;
    case "mongo_atlas_aws":
      return <AwsAccessFields access={auth.credentials} onChange={(credentials) => { setAuth({ ...auth, credentials }); }} {...secrets} />;
    case "mongo_atlas_oidc":
      return <OidcFields auth={auth} setAuth={setAuth} />;
    case "aws_credentials":
      return <AwsAccessFields access={auth.credentials} onChange={(credentials) => { setAuth({ ...auth, credentials }); }} {...secrets} />;
  }
}

// ---------------------------------------------------------------------------
// Secret field: write-only, "(unchanged)" when one is stored
// ---------------------------------------------------------------------------

interface SecretInputProps extends SecretProps {
  label: string;
  field: SecretKey;
  placeholder?: string | undefined;
  optional?: boolean;
  description?: string | undefined;
}

// WHAT:  null = untouched (keeps the stored secret), "" = remove it, text = replace it.
function SecretInput({ label, field, placeholder, optional = false, description, input, editing, patch, reveal, onToggleReveal }: SecretInputProps) {
  const current = input.authSecrets[field];
  const typed = current ?? "";
  const removing = current === "";
  const stored = editing?.hasAuthSecrets[field] === true && !removing;
  const set = (value: string | null) => { patch({ authSecrets: { ...input.authSecrets, [field]: value } }); };
  return (
    <Field
      label={label}
      type={reveal ? "text" : "password"}
      value={typed}
      onChange={(value) => { set(value === "" ? null : value); }}
      placeholder={removing ? "will be removed when you save" : stored && typed === "" ? "•••••••• (unchanged)" : placeholder}
      description={description}
      optional={optional}
      mono
      suffix={
        <span className="flex items-center">
          {stored && typed === "" ? <IconButton icon="trash" label="Remove the stored value" onClick={() => { set(""); }} /> : null}
          {removing ? <IconButton icon="x" label="Keep the stored value" onClick={() => { set(null); }} /> : null}
          <IconButton icon={reveal ? "eye-off" : "eye"} label={reveal ? "Hide secret" : "Show secret"} onClick={onToggleReveal} />
        </span>
      }
    />
  );
}

// ---------------------------------------------------------------------------
// AWS identity (RDS IAM, Atlas AWS, DynamoDB)
// ---------------------------------------------------------------------------

const AWS_SOURCES: readonly Option<AwsSourceKind>[] = [
  { value: "default_chain", label: "Default chain (environment, profile, SSO)" },
  { value: "profile", label: "Named profile" },
  { value: "access_keys", label: "Access keys" },
];

// WHAT:  Where AWS credentials come from, and an optional role to assume.
function AwsAccessFields({ access, onChange, withKeys = true, ...secrets }: SecretProps & { access: AwsAccess; onChange: (next: AwsAccess) => void; withKeys?: boolean }) {
  const source = access.source;
  const sources = withKeys ? AWS_SOURCES : AWS_SOURCES.filter((option) => option.value !== "access_keys");
  return (
    <>
      <AppSelect label="AWS credentials" value={source.source} options={sources} onChange={(kind) => { onChange({ ...access, source: sourceOfKind(kind, source) }); }} />
      {source.source === "default_chain" ? (
        <p className="-mt-3 text-xs text-muted">
          Reads AWS_ACCESS_KEY_ID, then the shared profile (AWS_PROFILE or default, including IAM Identity Center after <span className="font-mono">aws sso login</span> and credential_process), then web identity and container credentials.
        </p>
      ) : null}
      {source.source === "profile" ? (
        <Field label="Profile name" value={source.name} onChange={(name) => { onChange({ ...access, source: { source: "profile", name } }); }} placeholder="default" description="From ~/.aws/credentials and ~/.aws/config." mono />
      ) : null}
      {source.source === "access_keys" ? (
        <>
          <Field label="Access key ID" value={source.accessKeyId} onChange={(accessKeyId) => { onChange({ ...access, source: { source: "access_keys", accessKeyId } }); }} placeholder="AKIA…" mono />
          <SecretInput {...secrets} label="Secret access key" field="awsSecretAccessKey" />
          <SecretInput {...secrets} label="Session token" field="awsSessionToken" optional description="Only for temporary credentials." />
        </>
      ) : null}
      <Field
        label="Role to assume (ARN)"
        value={access.roleArn ?? ""}
        onChange={(roleArn) => { onChange({ ...access, roleArn: nullable(roleArn) }); }}
        placeholder="arn:aws:iam::123456789012:role/DatabaseAccess"
        optional
        mono
      />
      {(access.roleArn ?? "").trim() !== "" ? (
        <div className="grid grid-cols-2 gap-2">
          <Field label="Role session name" value={access.roleSessionName ?? ""} onChange={(roleSessionName) => { onChange({ ...access, roleSessionName: nullable(roleSessionName) }); }} placeholder="db-free" optional />
          <Field label="External ID" value={access.externalId ?? ""} onChange={(externalId) => { onChange({ ...access, externalId: nullable(externalId) }); }} optional mono />
        </div>
      ) : null}
    </>
  );
}

type RdsAuth = Extract<ConnectionAuth, { method: "aws_rds_iam" }>;

function RdsFields({ auth, setAuth, ...secrets }: SecretProps & { auth: RdsAuth; setAuth: (next: ConnectionAuth) => void }) {
  return (
    <>
      <Field
        label="AWS region"
        value={auth.region ?? ""}
        onChange={(region) => { setAuth({ ...auth, region: nullable(region) }); }}
        placeholder="read from the host name"
        description="Needed only when the host is not an *.rds.amazonaws.com name (a proxy or custom domain)."
        optional
        mono
      />
      <AwsAccessFields access={auth.credentials} onChange={(credentials) => { setAuth({ ...auth, credentials }); }} {...secrets} />
      <div className="flex w-full flex-col gap-1.5">
        <Label htmlFor="rds-ca-bundle">
          CA bundle (PEM)
          <span className="text-muted/70">(optional)</span>
        </Label>
        <Textarea
          id="rds-ca-bundle"
          rows={3}
          value={auth.caBundle ?? ""}
          onChange={(event) => { setAuth({ ...auth, caBundle: nullable(event.target.value) }); }}
          placeholder="Leave empty to use the bundled Amazon RDS global certificate authorities."
          className="text-xs"
        />
        <p className="text-xs text-muted">Used to verify the server when the TLS mode is Verify CA or Verify full.</p>
      </div>
    </>
  );
}

// ---------------------------------------------------------------------------
// Azure AD
// ---------------------------------------------------------------------------

const AZURE_MODES: readonly Option<AzureAdMode>[] = [
  { value: "default", label: "Default (environment, managed identity, Azure CLI)" },
  { value: "client_secret", label: "Service principal (client secret)" },
  { value: "managed_identity", label: "Managed identity" },
  { value: "username_password", label: "Username and password" },
  { value: "access_token", label: "Access token" },
];

type AzureAuth = Extract<ConnectionAuth, { method: "azure_ad" }>;

function AzureFields({ auth, setAuth, ...secrets }: SecretProps & { auth: AzureAuth; setAuth: (next: ConnectionAuth) => void }) {
  const tenant = (
    <Field label="Tenant ID" value={auth.tenantId ?? ""} onChange={(tenantId) => { setAuth({ ...auth, tenantId: nullable(tenantId) }); }} placeholder="00000000-0000-0000-0000-000000000000" mono />
  );
  const client = (optional: boolean) => (
    <Field label="Client ID" value={auth.clientId ?? ""} onChange={(clientId) => { setAuth({ ...auth, clientId: nullable(clientId) }); }} placeholder="00000000-0000-0000-0000-000000000000" optional={optional} mono />
  );
  return (
    <>
      <AppSelect label="Method" value={auth.mode} options={AZURE_MODES} onChange={(mode) => { setAuth({ ...auth, mode }); }} />
      {auth.mode === "default" || auth.mode === "managed_identity" ? client(true) : null}
      {auth.mode === "default" ? (
        <p className="-mt-3 text-xs text-muted">
          Tries AZURE_TENANT_ID / AZURE_CLIENT_ID / AZURE_CLIENT_SECRET, then a managed identity, then <span className="font-mono">az login</span>. The client ID picks a user-assigned managed identity.
        </p>
      ) : null}
      {auth.mode === "client_secret" ? (
        <>
          {tenant}
          {client(false)}
          <SecretInput {...secrets} label="Client secret" field="azureClientSecret" />
        </>
      ) : null}
      {auth.mode === "username_password" ? (
        <>
          {tenant}
          {client(false)}
          <Field
            label="Entra account"
            value={auth.username ?? ""}
            onChange={(username) => { setAuth({ ...auth, username: nullable(username) }); }}
            placeholder="the database user"
            description="The account that signs in. Leave empty to use the database user."
            optional
          />
          <SecretInput {...secrets} label="Account password" field="azurePassword" />
        </>
      ) : null}
      {auth.mode === "access_token" ? (
        <SecretInput {...secrets} label="Access token" field="azureAccessToken" placeholder="eyJ…" description="Paste a token from az account get-access-token. It is not renewed: paste a new one when it expires." />
      ) : null}
      <Field
        label="Token scope"
        value={auth.scope ?? ""}
        onChange={(scope) => { setAuth({ ...auth, scope: nullable(scope) }); }}
        placeholder="chosen for the engine"
        description="Only to override the default (ossrdbms-aad for PostgreSQL and MySQL, database.windows.net for SQL Server)."
        optional
        mono
      />
    </>
  );
}

// ---------------------------------------------------------------------------
// Google Cloud SQL IAM
// ---------------------------------------------------------------------------

const GCP_MODES: readonly Option<GcpAuthMode>[] = [
  { value: "adc", label: "Application Default Credentials" },
  { value: "service_account_key", label: "Service-account key" },
];

type GcpAuth = Extract<ConnectionAuth, { method: "gcp_cloud_sql_iam" }>;

function GcpFields({ auth, setAuth, ...secrets }: SecretProps & { auth: GcpAuth; setAuth: (next: ConnectionAuth) => void }) {
  return (
    <>
      <AppSelect label="Google credentials" value={auth.mode} options={GCP_MODES} onChange={(mode) => { setAuth({ ...auth, mode }); }} />
      {auth.mode === "adc" ? (
        <p className="-mt-3 text-xs text-muted">
          Uses GOOGLE_APPLICATION_CREDENTIALS, then <span className="font-mono">gcloud auth application-default login</span>, then the metadata server.
        </p>
      ) : (
        <SecretInput {...secrets} label="Service-account JSON key" field="gcpServiceAccountJson" placeholder="Paste the contents of the key file" />
      )}
      <Field
        label="Impersonate service account"
        value={auth.impersonateServiceAccount ?? ""}
        onChange={(impersonateServiceAccount) => { setAuth({ ...auth, impersonateServiceAccount: nullable(impersonateServiceAccount) }); }}
        placeholder="db-access@project.iam.gserviceaccount.com"
        description="Mint the token as this account (needs Service Account Token Creator on it)."
        optional
        mono
      />
    </>
  );
}

// ---------------------------------------------------------------------------
// MongoDB Atlas workload identity
// ---------------------------------------------------------------------------

const OIDC_PROVIDERS: readonly Option<OidcProvider>[] = [
  { value: "azure", label: "Azure managed identity" },
  { value: "gcp", label: "Google Cloud service account" },
  { value: "k8s", label: "Kubernetes service-account token" },
];

type OidcAuth = Extract<ConnectionAuth, { method: "mongo_atlas_oidc" }>;

function OidcFields({ auth, setAuth }: { auth: OidcAuth; setAuth: (next: ConnectionAuth) => void }) {
  return (
    <>
      <AppSelect label="Identity provider" value={auth.provider} options={OIDC_PROVIDERS} onChange={(provider) => { setAuth({ ...auth, provider }); }} />
      {auth.provider === "k8s" ? null : (
        <Field
          label="Token resource (audience)"
          value={auth.tokenResource ?? ""}
          onChange={(tokenResource) => { setAuth({ ...auth, tokenResource: nullable(tokenResource) }); }}
          placeholder="api://<your Atlas application ID>"
          mono
        />
      )}
    </>
  );
}

// ---------------------------------------------------------------------------
// The AWS-form engines (DynamoDB, QLDB, S3): region, endpoint, credentials
// ---------------------------------------------------------------------------

type KeysOrChain = "keys" | "profile" | "chain";

const AWS_CREDENTIAL_CHOICES: readonly Option<KeysOrChain>[] = [
  { value: "keys", label: "Access keys" },
  { value: "profile", label: "AWS profile" },
  { value: "chain", label: "Default credential chain" },
];

function credentialChoice(auth: ConnectionAuth): KeysOrChain {
  if (auth.method !== "aws_credentials") return "keys";
  return auth.credentials.source.source === "profile" ? "profile" : "chain";
}

function AwsForm({ input, editing, patch, reveal, onToggleReveal }: SecretProps) {
  const labels = fieldLabels(input.engine);
  const dynamo = input.engine === "dynamodb";
  const choice = credentialChoice(input.auth);
  const secrets: SecretProps = { input, editing, patch, reveal, onToggleReveal };

  const chooseCredentials = (next: KeysOrChain) => {
    if (next === credentialChoice(input.auth)) return;
    if (next === "keys") {
      patch({ auth: defaultAuth("password") });
      return;
    }
    const credentials: AwsAccess = { ...blankAwsAccess(), source: next === "profile" ? { source: "profile", name: "" } : { source: "default_chain" } };
    patch({ auth: { method: "aws_credentials", credentials } });
  };

  return (
    <div className="flex flex-col gap-3">
      <AppSelect label={labels.host} value={input.host ?? ""} options={regionOptions(input.host ?? "")} onChange={(host) => { patch({ host }); }} />
      <Field
        label={labels.database}
        value={input.database ?? ""}
        onChange={(database) => { patch({ database }); }}
        placeholder={dynamo ? "http://localhost:8000 (DynamoDB Local)" : ""}
        optional={dynamo}
      />
      {dynamo ? <AppSelect label="Credentials" value={choice} options={AWS_CREDENTIAL_CHOICES} onChange={chooseCredentials} /> : null}
      {choice === "keys" ? (
        <>
          <Field label={labels.username} value={input.username ?? ""} onChange={(username) => { patch({ username }); }} placeholder="AKIA…" mono />
          <Field
            label={labels.password}
            type={reveal ? "text" : "password"}
            value={input.password ?? ""}
            onChange={(password) => { patch({ password }); }}
            placeholder={editing && !input.password ? "•••••••• (unchanged)" : ""}
            mono
            suffix={<IconButton icon={reveal ? "eye-off" : "eye"} label={reveal ? "Hide secret" : "Show secret"} onClick={onToggleReveal} />}
          />
          <SecretInput {...secrets} label="Session token" field="awsSessionToken" optional description="Only for temporary credentials (STS, SSO)." />
        </>
      ) : null}
      {input.auth.method === "aws_credentials" ? (
        <AwsAccessFields
          {...secrets}
          withKeys={false}
          access={input.auth.credentials}
          onChange={(credentials) => { patch({ auth: { method: "aws_credentials", credentials } }); }}
        />
      ) : null}
      <p className="-mt-3 flex items-center gap-1.5 text-xs text-muted">
        <Icon name="lock" size={12} className="text-accent" />
        Requests are signed locally with SigV4; keys and tokens are stored sealed with AES-256-GCM and never leave the keychain unsealed.
      </p>
    </div>
  );
}
