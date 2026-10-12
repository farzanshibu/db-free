// SOT: connection-form, connection-editor-page, test-connection-flow, ssh-tunnel-form, connection-organisation-fields
import { useCallback, useEffect, useState } from "react";
import type { ConnectionFieldIssue, ConnectionInput, ConnectionSummary, Engine, Environment, SshAuth, SshTunnel, SslMode } from "@/lib/bindings";
import { inputFromSummary } from "@/lib/connectionGroups";
import { AuthSection } from "./AuthSection";
import { ColorSwatches } from "./ColorSwatches";
import { authShowsUser, authUsesPassword } from "@/lib/cloudAuth";
import { ENGINE_ORDER, blankInput, categoryLabel, cloudHostSslMode, engineForHost, engineMeta, engineNotice, fieldLabels, type EnginePreset, type ParsedConnection } from "@/lib/engines";
import { EngineIcon } from "@/components/global/EngineIcon";
import { ENVIRONMENT_ORDER, environmentMeta } from "@/lib/environments";
import { ipc, normalizeError } from "@/lib/ipc";
import { pickDirectory, pickPrivateKeyFile, pickSqliteFile } from "@/lib/native";
import { useWorkspace } from "@/stores/workspace";
import { IconButton } from "@/components/global/Button";
import { AppSelect, Field, Segmented, Toggle } from "@/components/global/Field";
import { Icon } from "@/lib/icons";
import { Alert, AlertContent, AlertDescription, AlertIndicator, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { ConfirmDialog } from "@/components/global/ConfirmDialog";
import { ConnectionOptionsFields } from "./ConnectionOptionsFields";
import { ConnectionStringBox } from "./ConnectionStringBox";
import { DatabaseDropZone } from "./DatabaseDropZone";
import { FieldIssue } from "./FieldIssue";
import { FolderPicker, TagPicker } from "./OrgPicker";
import { SshConfigImport } from "./SshConfigImport";
import { TlsFilesSection } from "./TlsFilesSection";
import { issueFor, mergeParsed, sshFromConfigHost } from "./formHelpers";
import { useConnectionOrg } from "./useConnectionOrg";

const SSL_MODES: readonly { value: SslMode; label: string }[] = [
  { value: "disable", label: "Disable" },
  { value: "prefer", label: "Prefer" },
  { value: "require", label: "Require" },
  { value: "verify_ca", label: "Verify CA" },
  { value: "verify_full", label: "Verify full" },
];

const SSH_AUTH: readonly { value: SshAuth; label: string }[] = [
  { value: "password", label: "Password" },
  { value: "private_key", label: "Private key" },
];

type Section = "general" | "ssl" | "ssh";

function asSection(value: string): Section {
  return value === "ssl" || value === "ssh" ? value : "general";
}

type Status = { tone: "ok" | "error"; text: string } | null;

// WHAT:  Full-page connection editor (DB Manager layout): centered column on a grid
//        background, Back in the title strip, Test / Save footer.
// HOW:   Keyed by the connection being edited so state initialises from props
//        without an effect.
export function ConnectionForm() {
  const page = useWorkspace((s) => s.page);
  const connections = useWorkspace((s) => s.connections);
  if (page.kind !== "connection-form") return null;
  const editing = page.editingId === null ? null : (connections.find((c) => c.id === page.editingId) ?? null);
  return <ConnectionFormBody key={editing?.id ?? `new:${page.preset?.id ?? ""}:${page.draft?.engine ?? ""}`} editing={editing} preset={page.preset} draft={page.draft} />;
}

function ConnectionFormBody({ editing, preset, draft }: { editing: ConnectionSummary | null; preset?: EnginePreset | undefined; draft?: ConnectionInput | undefined }) {
  const goConnections = useWorkspace((s) => s.goConnections);
  const goPicker = useWorkspace((s) => s.goPicker);
  const saveConnection = useWorkspace((s) => s.saveConnection);
  const deleteConnection = useWorkspace((s) => s.deleteConnection);
  const showInfo = useWorkspace((s) => s.showInfo);

  const [input, setInput] = useState<ConnectionInput>(() => (editing ? inputFromSummary(editing) : (draft ?? blankInput("postgres", preset))));
  const [status, setStatus] = useState<Status>(null);
  const [testing, setTesting] = useState(false);
  const [saving, setSaving] = useState(false);
  const [showPassword, setShowPassword] = useState(false);
  const [section, setSection] = useState<Section>("general");
  const [issues, setIssues] = useState<ConnectionFieldIssue[]>([]);
  // Field messages appear after the first Test / Save attempt, then follow the input.
  const [attempted, setAttempted] = useState(false);
  const [confirmingDelete, setConfirmingDelete] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [savingAndConnecting, setSavingAndConnecting] = useState(false);
  const connectNow = useWorkspace((s) => s.connect);
  const openForm = useWorkspace((s) => s.openForm);
  const refreshConnections = useWorkspace((s) => s.refreshConnections);
  const orgLoaded = useConnectionOrg((s) => s.loaded);
  const loadOrg = useConnectionOrg((s) => s.load);

  // The tag and folder pickers read the shared list; make sure it is loaded.
  useEffect(() => {
    if (!orgLoaded) void loadOrg();
  }, [orgLoaded, loadOrg]);

  // WHAT:  After a first attempt, re-validate as the input changes so a fixed field clears its message.
  // WHY:   The rules live in Rust (`ConnectionInput::check`); this asks, debounced, instead of copying them.
  useEffect(() => {
    if (!attempted) return;
    let cancelled = false;
    const timer = window.setTimeout(() => {
      ipc("validate_connection", { id: editing?.id ?? null, input })
        .then((found) => {
          if (!cancelled) setIssues(found);
        })
        .catch(() => undefined);
    }, 300);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [attempted, input, editing?.id]);

  const patch = (partial: Partial<ConnectionInput>) => setInput((prev) => ({ ...prev, ...partial }));
  // WHAT:  Edits the SSH block. Moving the tunnel to another host or port drops
  //        the pinned key: it belonged to the old server.
  const patchSsh = (partial: Partial<SshTunnel>) =>
    setInput((prev) => {
      const moved = (partial.host !== undefined && partial.host !== prev.ssh.host) || (partial.port !== undefined && partial.port !== prev.ssh.port);
      return { ...prev, ssh: { ...prev.ssh, ...(moved ? { hostKey: null } : {}), ...partial } };
    });
  const meta = engineMeta(input.engine);
  const labels = fieldLabels(input.engine);

  const changeEngine = (engine: Engine) => {
    const next = blankInput(engine);
    setInput((prev) => ({
      ...next,
      name: prev.name,
      environment: prev.environment,
      readOnly: prev.readOnly,
      ssh: prev.ssh,
      sshSecret: prev.sshSecret,
      folder: prev.folder,
      color: prev.color,
      favorite: prev.favorite,
      extra: { ...next.extra, tagIds: prev.extra.tagIds, folderIds: prev.extra.folderIds },
    }));
  };
  // WHAT:  A connection string was pasted into (or typed in) the form's box.
  const applyParsed = (parsed: ParsedConnection, force: boolean) => { setInput((prev) => mergeParsed(prev, parsed, force)); };
  // WHAT:  The host is a cloud address: raise a weak TLS default to `require`.
  // WHY:   Supabase, Neon, RDS, Azure and Atlas refuse plaintext; the default
  //        `prefer` only works by luck of the fallback.
  const changeHost = (host: string) =>
    setInput((prev) => {
      const cloud = cloudHostSslMode(host);
      const raise = cloud !== null && (prev.sslMode === "prefer" || prev.sslMode === "disable") && engineMeta(prev.engine).form === "server";
      return { ...prev, host, ...(raise ? { sslMode: cloud } : {}) };
    });
  // A host that belongs to a hosted preset (a *.supabase.co address typed into a Postgres form).
  const suggestedEngine = engineForHost(input.engine, input.host);
  const useSuggestedEngine = () => { setInput((prev) => ({ ...prev, engine: suggestedEngine, sslMode: engineMeta(suggestedEngine).sslMode ?? prev.sslMode })); };
  const notice = engineNotice(input.engine);
  // Dropping a database file on the form fills its path (and names the connection if it has no name yet).
  const fillFilePath = useCallback((filePath: string) => {
    setInput((prev) => ({ ...prev, filePath, name: prev.name.trim().length > 0 ? prev.name : (filePath.split(/[\\/]/).pop() ?? prev.name) }));
  }, []);
  const changeEnvironment = (environment: Environment) => patch({ environment, readOnly: environmentMeta(environment).readOnlyDefault });

  // WHAT:  Asks Rust whether the input is acceptable and paints each problem under its field.
  // WHY:   `ConnectionInput::check` names the field of every problem, so a form with
  //        three mistakes shows all three at once instead of one per attempt.
  const validateNow = async (): Promise<boolean> => {
    const found = await ipc("validate_connection", { id: editing?.id ?? null, input });
    setIssues(found);
    setAttempted(true);
    if (found.length === 0) return true;
    setStatus({ tone: "error", text: found.length === 1 ? (found[0]?.message ?? "Check the highlighted field.") : `${found.length} fields need attention.` });
    return false;
  };

  const test = async () => {
    setTesting(true);
    setStatus(null);
    try {
      if (!(await validateNow())) return;
      await ipc("test_connection", { id: editing?.id ?? null, input });
      setStatus({ tone: "ok", text: "Connection succeeded." });
    } catch (raw) {
      setStatus({ tone: "error", text: normalizeError(raw).message });
    } finally {
      setTesting(false);
    }
  };

  // WHAT:  Save, and with `connectAfter` open the connection straight away ("Save & connect").
  // HOW:   Navigates back to the list first, so a failed connect lands in the
  //        "Connection failed" dialog there, with Retry and Edit.
  const save = async (connectAfter: boolean) => {
    if (connectAfter) setSavingAndConnecting(true);
    else setSaving(true);
    setStatus(null);
    try {
      if (!(await validateNow())) return;
      const saved = await saveConnection(editing?.id ?? null, input);
      showInfo(`Saved "${saved.name}".`);
      goConnections();
      if (connectAfter) await connectNow(saved.id);
    } catch (raw) {
      setStatus({ tone: "error", text: normalizeError(raw).message });
    } finally {
      setSaving(false);
      setSavingAndConnecting(false);
    }
  };

  const remove = async () => {
    if (!editing) return;
    setDeleting(true);
    try {
      await deleteConnection(editing.id);
      setConfirmingDelete(false);
      goConnections();
    } catch (raw) {
      setConfirmingDelete(false);
      setStatus({ tone: "error", text: normalizeError(raw).message });
    } finally {
      setDeleting(false);
    }
  };

  // WHAT:  "Duplicate": a "(copy)" of this connection, with its sealed password and SSH secret, opened for editing.
  const duplicate = async () => {
    if (!editing) return;
    try {
      const copy = await ipc("duplicate_connection", { id: editing.id });
      await refreshConnections();
      showInfo(`Created "${copy.name}".`);
      openForm(copy.id);
    } catch (raw) {
      setStatus({ tone: "error", text: normalizeError(raw).message });
    }
  };

  const browseKey = async () => {
    const path = await pickPrivateKeyFile();
    if (path) patchSsh({ keyPath: path });
  };
  const savedSshSecret = editing?.hasSshSecret === true && !input.sshSecret;

  const browse = async () => {
    const path = input.engine === "rocksdb" ? await pickDirectory() : await pickSqliteFile();
    if (path) patch({ filePath: path });
  };

  return (
    <div className="grid-bg flex h-full min-h-0 flex-1 flex-col">
      <div className="drag-region flex app-titlebar app-pad-x shrink-0 items-center gap-2 border-b border-border/40 glass-header" data-tauri-drag-region>
        <Button variant="ghost" size="sm" onClick={editing ? goConnections : goPicker} className="rounded-md text-muted hover:bg-surface-secondary/70 hover:text-foreground liquid-hover">
          <Icon name="chevron-left" size={14} />
          Back
        </Button>
        <span className="text-[13px] font-semibold text-foreground tracking-tight" data-tauri-drag-region>
          {editing ? "Edit Connection" : preset ? `New ${preset.label} Connection` : "New Connection"}
        </span>
        <div className="drag-region h-full flex-1" data-tauri-drag-region />
      </div>

      <ScrollArea className="min-h-0 flex-1">
        <div className="mx-auto flex w-full max-w-[640px] flex-col gap-3 px-4 pt-3 pb-4">
          <Card className="glass-card rounded-lg p-3 shadow-xs flex flex-col gap-3 border-border/40">
            <CardContent className="flex flex-col gap-3 p-0">
              <AppSelect label="Type" value={input.engine} options={ENGINE_ORDER.map((e) => ({ value: e, label: `${engineMeta(e).label} · ${categoryLabel(engineMeta(e).kind)}`, leading: <EngineIcon engine={e} size={14} /> }))} onChange={changeEngine} />
              {notice !== null ? (
                <Alert variant={notice.tone === "warning" ? "warning" : "default"} className="rounded-lg">
                  <AlertIndicator />
                  <AlertContent>
                    <AlertDescription>{notice.text}</AlertDescription>
                  </AlertContent>
                </Alert>
              ) : null}
              <ConnectionStringBox engine={input.engine} onApply={applyParsed} />
              <Field label="Connection Name" value={input.name} onChange={(name) => patch({ name })} placeholder="local-db" autoFocus />
              <FieldIssue message={issueFor(issues, "name")} />
              <AppSelect label="Environment" value={input.environment} options={ENVIRONMENT_ORDER.map((e) => ({ value: e, label: environmentMeta(e).label }))} onChange={changeEnvironment} />
              <FolderPicker selected={input.extra.folderIds} onChange={(folderIds) => { patch({ extra: { ...input.extra, folderIds } }); }} />
              <TagPicker selected={input.extra.tagIds} onChange={(tagIds) => { patch({ extra: { ...input.extra, tagIds } }); }} />
              <ColorSwatches value={input.color} onChange={(color) => patch({ color })} />

              {meta.form === "file" ? (
                <div className="flex flex-col gap-2">
                  <Field
                    label={labels.filePath}
                    value={input.filePath ?? ""}
                    onChange={(filePath) => patch({ filePath })}
                    placeholder={input.engine === "rocksdb" ? "/path/to/rocksdb-dir" : input.engine === "duckdb" ? "/path/to/analytics.duckdb" : "/path/to/database.sqlite"}
                    suffix={<Button variant="secondary" size="sm" onClick={() => void browse()} className="rounded-md">Browse…</Button>}
                  />
                  <FieldIssue message={issueFor(issues, "file_path")} />
                  <DatabaseDropZone onPath={fillFilePath} noun={input.engine === "rocksdb" ? "RocksDB directory" : "database file"} />
                </div>
              ) : meta.form === "http_token" ? (
                <div className="flex flex-col gap-3">
                  <Field label={labels.host} value={input.host ?? ""} onChange={(host) => patch({ host })} placeholder={meta.hostPlaceholder ?? (meta.defaultPort !== null ? `http://localhost:${meta.defaultPort}` : "https://…")} mono />
                  {meta.fields?.database !== undefined || meta.defaultDatabase.length > 0 ? (
                    <Field label={labels.database} value={input.database ?? ""} onChange={(database) => patch({ database })} placeholder={meta.defaultDatabase.length > 0 ? meta.defaultDatabase : ""} optional={input.engine !== "cloudflare_d1"} />
                  ) : null}
                  {meta.fields?.username !== undefined ? (
                    <Field label={labels.username} value={input.username ?? ""} onChange={(username) => patch({ username })} placeholder={meta.defaultUser} optional />
                  ) : null}
                  <Field
                    label={labels.password}
                    type={showPassword ? "text" : "password"}
                    value={input.password ?? ""}
                    onChange={(password) => patch({ password })}
                    placeholder={editing && !input.password ? "•••••••• (unchanged)" : ""}
                    optional
                    suffix={<IconButton icon={showPassword ? "eye-off" : "eye"} label={showPassword ? "Hide secret" : "Show secret"} onClick={() => setShowPassword((v) => !v)} />}
                  />
                  <AppSelect label="TLS" value={input.sslMode} options={SSL_MODES} onChange={(sslMode) => patch({ sslMode })} />
                  <p className="-mt-3 flex items-center gap-1.5 text-xs text-muted">
                    <Icon name="lock" size={12} className="text-accent" />
                    Keychain enabled — your {meta.label} secret is stored with AES-256-GCM.
                  </p>
                </div>
              ) : meta.form === "aws" ? (
                <AuthSection input={input} editing={editing} patch={patch} />
              ) : meta.form === "gcp" ? (
                <div className="flex flex-col gap-3">
                  <Field label={labels.database} value={input.database ?? ""} onChange={(database) => patch({ database })} placeholder="my-gcp-project" mono />
                  <Field label={labels.username} value={input.username ?? ""} onChange={(username) => patch({ username })} placeholder={input.engine === "firestore" ? "(default)" : "dataset_name"} optional />
                  <Field
                    label={labels.password}
                    type={showPassword ? "text" : "password"}
                    value={input.password ?? ""}
                    onChange={(password) => patch({ password })}
                    placeholder={editing && !input.password ? "•••••••• (unchanged)" : "Paste the service-account JSON key, or an OAuth access token"}
                    mono
                    suffix={<IconButton icon={showPassword ? "eye-off" : "eye"} label={showPassword ? "Hide secret" : "Show secret"} onClick={() => setShowPassword((v) => !v)} />}
                  />
                  <Field label="Emulator host" value={input.host ?? ""} onChange={(host) => patch({ host })} placeholder="localhost:8080 (leave empty for Google Cloud)" optional mono />
                  <p className="-mt-3 flex items-center gap-1.5 text-xs text-muted">
                    <Icon name="lock" size={12} className="text-accent" />
                    A short-lived access token is minted locally from the service-account key (RS256 JWT).
                  </p>
                </div>
              ) : (
                <Tabs value={section} onValueChange={(next) => { setSection(asSection(next)); }} className="w-full">
                  <TabsList className="glass-pill border border-border/40 p-0.5">
                    <TabsTrigger value="general">General</TabsTrigger>
                    <TabsTrigger value="ssl">SSL / TLS</TabsTrigger>
                    <TabsTrigger value="ssh">
                      SSH
                      {input.ssh.enabled ? <span className="size-1.5 rounded-full bg-accent" aria-label="tunnel on" /> : null}
                    </TabsTrigger>
                  </TabsList>
                  <TabsContent value="general" className="flex flex-col gap-3 pt-2.5">
                    <div className={input.extra.srv ? "" : "grid grid-cols-[1fr_120px] gap-2"}>
                      <Field label={input.extra.srv ? "Cluster host (SRV name)" : labels.host} value={input.host ?? ""} onChange={changeHost} placeholder={input.extra.srv ? "cluster0.abc12.mongodb.net" : (meta.hostPlaceholder ?? "localhost")} />
                      {input.extra.srv ? null : (
                        <Field label={labels.port} type="number" value={input.port !== null ? String(input.port) : ""} onChange={(port) => patch({ port: port === "" ? null : Number(port) })} placeholder={String(meta.defaultPort ?? "")} />
                      )}
                    </div>
                    <FieldIssue message={issueFor(issues, "host") ?? issueFor(issues, "port")} />
                    {suggestedEngine !== input.engine ? (
                      <Alert variant="default" className="rounded-lg">
                        <AlertIndicator />
                        <AlertContent className="gap-2">
                          <AlertDescription>This address belongs to {engineMeta(suggestedEngine).label}. Its preset sets the right TLS mode and shows its connection tips.</AlertDescription>
                          <div>
                            <Button variant="secondary" size="sm" onClick={useSuggestedEngine}>
                              Use the {engineMeta(suggestedEngine).label} preset
                            </Button>
                          </div>
                        </AlertContent>
                      </Alert>
                    ) : null}
                    <Field label={labels.database} value={input.database ?? ""} onChange={(database) => patch({ database })} placeholder={meta.defaultDatabase.length > 0 ? meta.defaultDatabase : "mydb"} optional />
                    <FieldIssue message={issueFor(issues, "database")} />
                    <AuthSection input={input} editing={editing} patch={patch} />
                    {authShowsUser(input) ? <Field label={labels.username} value={input.username ?? ""} onChange={(username) => patch({ username })} placeholder={meta.defaultUser} optional={meta.defaultUser.length === 0} /> : null}
                    <FieldIssue message={issueFor(issues, "username")} />
                    {authUsesPassword(input) ? (
                      <>
                        <Field
                          label={labels.password}
                          type={showPassword ? "text" : "password"}
                          value={input.password ?? ""}
                          onChange={(password) => patch({ password })}
                          placeholder={editing && !input.password ? "•••••••• (unchanged)" : "secret"}
                          suffix={<IconButton icon={showPassword ? "eye-off" : "eye"} label={showPassword ? "Hide password" : "Show password"} onClick={() => setShowPassword((v) => !v)} />}
                        />
                        <p className="-mt-3 flex items-center gap-1.5 text-xs text-muted">
                          <Icon name="lock" size={12} className="text-accent" />
                          Keychain enabled — stored with AES-256-GCM; the key never leaves your OS keychain.
                        </p>
                      </>
                    ) : null}
                    <ConnectionOptionsFields input={input} patch={patch} issues={issues} />
                  </TabsContent>
                  <TabsContent value="ssl" className="flex flex-col gap-3 pt-2.5">
                    <AppSelect label="SSL mode" value={input.sslMode} options={SSL_MODES} onChange={(sslMode) => patch({ sslMode })} />
                    {input.ssh.enabled ? (
                      <p className="-mt-2 flex items-start gap-1.5 text-xs text-muted">
                        <Icon name="info" size={12} className="mt-0.5 shrink-0 text-accent" />
                        Through the SSH tunnel SQL Server checks the certificate against the real server name. PostgreSQL and MySQL check the certificate chain but cannot check the name, so “Verify full” runs as “Verify CA”.
                      </p>
                    ) : null}
                    <TlsFilesSection input={input} patch={patch} issues={issues} />
                  </TabsContent>
                  {/* WHAT:  SSH tunnel through a bastion host. The database host
                      and port on the General tab are resolved *from the SSH
                      host*, so a private address like 10.0.0.5 works here. */}
                  <TabsContent value="ssh" className="flex flex-col gap-3 pt-2.5">
                    <Toggle
                      checked={input.ssh.enabled}
                      onChange={(enabled) => patchSsh({ enabled })}
                      label="Connect through an SSH tunnel"
                      description="The database host above is reached from the SSH server, not from this machine."
                    />
                    {input.ssh.enabled ? (
                      <>
                        <SshConfigImport onPick={(host) => { setInput((prev) => ({ ...prev, ssh: sshFromConfigHost(prev.ssh, host) })); }} />
                        <div className="grid grid-cols-[1fr_120px] gap-2">
                          <Field label="SSH host" value={input.ssh.host ?? ""} onChange={(host) => patchSsh({ host })} placeholder="bastion.example.com" mono />
                          <Field label="SSH port" type="number" value={String(input.ssh.port)} onChange={(port) => patchSsh({ port: port === "" ? 22 : Number(port) })} placeholder="22" />
                        </div>
                        <FieldIssue message={issueFor(issues, "ssh_host") ?? issueFor(issues, "ssh_port")} />
                        <Field label="SSH user" value={input.ssh.user ?? ""} onChange={(user) => patchSsh({ user })} placeholder="ubuntu" />
                        <FieldIssue message={issueFor(issues, "ssh_user")} />
                        <Segmented label="Authentication" value={input.ssh.auth} options={SSH_AUTH} onChange={(auth) => patchSsh({ auth })} />
                        {input.ssh.auth === "private_key" ? (
                          <Field
                            label="Private key file"
                            value={input.ssh.keyPath ?? ""}
                            onChange={(keyPath) => patchSsh({ keyPath })}
                            placeholder="~/.ssh/id_ed25519"
                            mono
                            suffix={<Button variant="secondary" size="sm" onClick={() => void browseKey()} className="rounded-md">Browse…</Button>}
                          />
                        ) : null}
                        {input.ssh.auth === "private_key" ? <FieldIssue message={issueFor(issues, "ssh_key_path")} /> : null}
                        <Field
                          label={input.ssh.auth === "private_key" ? "Key passphrase" : "SSH password"}
                          type={showPassword ? "text" : "password"}
                          value={input.sshSecret ?? ""}
                          onChange={(sshSecret) => patch({ sshSecret })}
                          placeholder={savedSshSecret ? "•••••••• (unchanged)" : input.ssh.auth === "private_key" ? "only if the key is encrypted" : "secret"}
                          optional={input.ssh.auth === "private_key"}
                          suffix={<IconButton icon={showPassword ? "eye-off" : "eye"} label={showPassword ? "Hide secret" : "Show secret"} onClick={() => setShowPassword((v) => !v)} />}
                        />
                        <Field
                          label="Host key fingerprint"
                          value={input.ssh.hostKey ?? ""}
                          onChange={(hostKey) => patchSsh({ hostKey: hostKey.trim() === "" ? null : hostKey })}
                          placeholder="Pinned on first connect (SHA256:…)"
                          description="The SSH server must present this key. Clear it to trust a server that was re-keyed."
                          optional
                          mono
                        />
                        <p className="-mt-3 flex items-center gap-1.5 text-xs text-muted">
                          <Icon name="lock" size={12} className="text-accent" />
                          The SSH password or passphrase is sealed with AES-256-GCM, like the database password.
                        </p>
                      </>
                    ) : null}
                  </TabsContent>
                </Tabs>
              )}

              <Toggle checked={input.readOnly} onChange={(readOnly) => patch({ readOnly })} label="Read-only lock" description="Blocks every write and DDL statement on this connection. On by default for Production." />
              <Toggle checked={input.favorite} onChange={(favorite) => patch({ favorite })} label="Favourite" description="Listed first, on the connections page and in the sidebar switcher." />

              {status ? (
                <Alert variant={status.tone === "ok" ? "success" : "danger"} className="rounded-lg font-mono text-xs">
                  <AlertIndicator />
                  <AlertContent>
                    <AlertTitle className="font-sans font-semibold">{status.tone === "ok" ? "Success" : "Connection Error"}</AlertTitle>
                    <AlertDescription className="selectable">{status.text}</AlertDescription>
                  </AlertContent>
                </Alert>
              ) : null}
            </CardContent>
          </Card>
        </div>
      </ScrollArea>

      <div className="flex shrink-0 items-center justify-center gap-2 border-t border-border/40 glass-header px-3 py-1.5">
        {editing ? (
          <div className="mr-auto flex items-center gap-2">
            <Button variant="danger-soft" onClick={() => { setConfirmingDelete(true); }} className="rounded-md liquid-hover">
              Delete
            </Button>
            <Button variant="tertiary" onClick={() => void duplicate()} className="rounded-md liquid-hover">
              <Icon name="copy" size={13} />
              Duplicate
            </Button>
          </div>
        ) : null}
        <Button variant="secondary" pending={testing} onClick={() => void test()} className="w-[140px] rounded-md glass-pill text-foreground liquid-hover">
          Test Connection
        </Button>
        <Button variant="secondary" pending={saving} disabled={savingAndConnecting} onClick={() => void save(false)} className="w-[140px] rounded-md glass-pill text-foreground liquid-hover">
          {editing ? "Save Changes" : "Save"}
        </Button>
        <Button pending={savingAndConnecting} disabled={saving} onClick={() => void save(true)} className="w-[150px] rounded-md font-semibold liquid-hover">
          Save &amp; Connect
        </Button>
      </div>
      <ConfirmDialog
        open={confirmingDelete}
        onOpenChange={setConfirmingDelete}
        title="Delete connection?"
        description="The connection and its saved credentials are removed. Query history for it goes too."
        confirmLabel="Delete connection"
        requireText={editing?.name ?? ""}
        pending={deleting}
        onConfirm={remove}
      />
    </div>
  );
}
