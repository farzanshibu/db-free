// SOT: ipc-client, invoke-wrapper, command-map, app-error-normalization, client-block
import { Channel, invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AgentChatIdRequest,
  AgentChatRequest,
  AgentDecisionRequest,
  AgentEvent,
  AgentRunRequest,
  AgentSkill,
  AgentTurn,
  AiGenerateRequest,
  AiReply,
  AppError,
  AppSettings,
  BackupEvent,
  BackupReport,
  BackupRequest,
  BackupRunRequest,
  BackupSupport,
  BufferIdRequest,
  CancelQueryRequest,
  CatalogRequest,
  ChangePreview,
  ChangesRequest,
  ClearHistoryRequest,
  ColumnInfo,
  ColumnsRequest,
  CommandName,
  CompareTableDataRequest,
  ConnectRequest,
  ConnectionIdRequest,
  ConnectionSummary,
  CreateTemplateRequest,
  DataCompare,
  Document,
  DownloadObjectReport,
  DetectToolsRequest,
  DownloadObjectRequest,
  EditorBuffer,
  ExecuteQueryRequest,
  ExplainRequest,
  ExportReport,
  ExportRequest,
  ForeignKey,
  HistoryEntry,
  HistoryRequest,
  IdRequest,
  ImportReport,
  ImportRequest,
  ListDocumentsRequest,
  ObjectDetail,
  ObjectRequest,
  ObjectSummary,
  ObjectsRequest,
  PlanReport,
  QueryOutcome,
  RangeQueryCommand,
  RangeResult,
  RestoreRequest,
  ResultSet,
  RunWorkflowRequest,
  SaveBufferRequest,
  SaveConnectionRequest,
  SaveDocumentRequest,
  SaveQueryRequest,
  SaveSettingsRequest,
  SaveSqlFileRequest,
  SchemaDiff,
  SchemaDiffRequest,
  SavedQuery,
  SchemaCatalog,
  SearchCommand,
  SearchResult,
  ServerStats,
  SessionInfo,
  SessionRequest,
  SplitScriptRequest,
  StatementSpan,
  TablePage,
  TablePageRequest,
  UpdateProgress,
  UpdateStatus,
  VectorSearchCommand,
  WorkflowRunReport,
} from "./bindings";
import type {
  CancelWorkflowRequest,
  LastNodeConfigRequest,
  NodeConfig,
  RegexTestResult,
  TestRegexRequest,
  WorkflowEvent,
  WorkflowScheduleStatus,
} from "./bindings";
import type {
  AutosaveDocumentRequest,
  CheckpointDocumentRequest,
  DiagramCustomization,
  DiagramDdl,
  DiagramDdlRequest,
  DiagramScopeRequest,
  DocumentHistoryRequest,
  DocumentVersionRequest,
  DocumentVersionSummary,
  SaveDiagramCustomizationsRequest,
  SaveDiagramExportRequest,
} from "./bindings";
import type {
  CreateFolderRequest,
  DuplicateDocumentRequest,
  DuplicateQueryRequest,
  ExportQueriesRequest,
  HomeEntityRequest,
  HomePin,
  HomeRecent,
  LibraryFolder,
  LibraryLayout,
  MoveFolderRequest,
  MoveItemRequest,
  RenameRequest,
  ReorderPinsRequest,
  SavedQueryVersion,
  UnpinRequest,
} from "./bindings";
import type { CommitReport, DeleteFkRemapRequest, FetchRowsRequest, FkRemap, RowsSqlRequest, SaveFkRemapRequest } from "./bindings";
import type {
  DdlPreview,
  DdlProfile,
  ListTagsRequest,
  MoveTaggedTableRequest,
  PreviewDdlRequest,
  ReorderTagsRequest,
  SaveTagRequest,
  TableSpec,
  TableTag,
  TagCollapseRequest,
  TagState,
  TagTableRequest,
} from "./bindings";
import type { BatchEvent, BatchSummary, EditorHistoryRequest, ExecuteBatchRequest, HistoryPage, HistoryPageRequest, OpenedSqlFile, ReadSqlFileRequest } from "./bindings";
import type {
  ExportProgress,
  ImportPreview,
  ImportProgress,
  JsonImportPlan,
  JsonImportReport,
  JsonImportRequest,
  JsonPlanRequest,
  KeyInfoRequest,
  PreviewExportRequest,
  PreviewImportRequest,
  RedisKeyInfo,
  RedisKeyPage,
  ScanKeysRequest,
  SqlFileInfo,
  SqlFileRequest,
  SqlImportProgress,
  SqlImportReport,
  SqlImportRequest,
  ExportPreview,
} from "./bindings";
import type {
  ConnectionExportReport,
  ConnectionFieldIssue,
  ConnectionFolder,
  ConnectionImportReport,
  ConnectionOrg,
  ConnectionTag,
  CredentialStatus,
  ExportConnectionsRequest,
  ImportConnectionsRequest,
  OpenDatabaseFileRequest,
  OrgIdRequest,
  SaveConnectionFolderRequest,
  SaveConnectionTagRequest,
  SaveTransientRequest,
  SetConnectionOrgRequest,
  SshConfigHost,
} from "./bindings";
import type {
  AppInfo,
  ExportThemeRequest,
  ImportThemeRequest,
  LogUiErrorRequest,
  MenuShortcutsRequest,
  OpenExternalRequest,
  OpenWindowRequest,
  SetWindowThemeRequest,
  SetZoomRequest,
} from "./bindings";
import type {
  AiChatDetail,
  AiChatRenameRequest,
  AiChatSummary,
  AiChatsRequest,
  AiModelList,
  AiModelsRequest,
  AiTestRequest,
  AiTestResult,
  DiagramFromSchemaRequest,
  SaveAiSettingsRequest,
} from "./bindings";
import type { CloseSessionRequest, SchemaCacheUpdated } from "./bindings";

// WHAT:  The client-side block. Every call into the Rust core goes through `ipc`.
// WHY:   One place normalises errors into AppError, so components switch on
//        `kind` and never parse strings. ESLint + guardrail.py forbid `invoke`
//        anywhere else.
// HOW:   CommandMap is checked in both directions against the Rust CommandName
//        enum: a command added in Rust without a signature here fails `tsc`.
// WHERE: src-tauri/src/commands/mod.rs (CommandName), src/lib/bindings (types)
interface CommandMap {
  list_connections: { req: undefined; res: ConnectionSummary[] };
  save_connection: { req: SaveConnectionRequest; res: ConnectionSummary };
  delete_connection: { req: ConnectionIdRequest; res: null };
  test_connection: { req: SaveConnectionRequest; res: null };
  connect: { req: ConnectRequest; res: ConnectionSummary };
  disconnect: { req: ConnectionIdRequest; res: null };
  active_sessions: { req: undefined; res: string[] };
  describe_session: { req: SessionRequest; res: SessionInfo };
  ping_session: { req: SessionRequest; res: number };
  load_catalog: { req: CatalogRequest; res: SchemaCatalog };
  load_columns: { req: ColumnsRequest; res: ColumnInfo[] };
  load_foreign_keys: { req: CatalogRequest; res: ForeignKey[] };
  load_ddl: { req: ColumnsRequest; res: string | null };
  fetch_table_page: { req: TablePageRequest; res: TablePage };
  execute_query: { req: ExecuteQueryRequest; res: QueryOutcome };
  list_history: { req: HistoryRequest; res: HistoryEntry[] };
  clear_history: { req: ClearHistoryRequest; res: number };
  list_buffers: { req: undefined; res: EditorBuffer[] };
  save_buffer: { req: SaveBufferRequest; res: EditorBuffer };
  delete_buffer: { req: BufferIdRequest; res: null };
  split_script: { req: SplitScriptRequest; res: StatementSpan[] };
  save_sql_file: { req: SaveSqlFileRequest; res: string };
  get_settings: { req: undefined; res: AppSettings };
  save_settings: { req: SaveSettingsRequest; res: AppSettings };
  list_saved_queries: { req: undefined; res: SavedQuery[] };
  save_saved_query: { req: SaveQueryRequest; res: SavedQuery };
  delete_saved_query: { req: IdRequest; res: null };
  list_documents: { req: ListDocumentsRequest; res: Document[] };
  save_document: { req: SaveDocumentRequest; res: Document };
  delete_document: { req: IdRequest; res: null };
  preview_changes: { req: ChangesRequest; res: ChangePreview };
  commit_changes: { req: ChangesRequest; res: CommitReport };
  export_tables: { req: ExportRequest; res: ExportReport };
  import_file: { req: ImportRequest; res: ImportReport };
  ai_generate: { req: AiGenerateRequest; res: AiReply };
  explain_query: { req: ExplainRequest; res: PlanReport };
  run_workflow: { req: RunWorkflowRequest; res: WorkflowRunReport };
  list_objects: { req: ObjectsRequest; res: ObjectSummary[] };
  load_object: { req: ObjectRequest; res: ObjectDetail };
  server_stats: { req: SessionRequest; res: ServerStats };
  vector_search: { req: VectorSearchCommand; res: ResultSet };
  search_documents: { req: SearchCommand; res: SearchResult };
  query_range: { req: RangeQueryCommand; res: RangeResult };
  load_history: { req: ObjectRequest; res: ResultSet };
  download_object: { req: DownloadObjectRequest; res: DownloadObjectReport };
  create_template: { req: CreateTemplateRequest; res: string | null };
  check_update: { req: undefined; res: UpdateStatus };
  download_update: { req: undefined; res: UpdateStatus };
  install_update: { req: undefined; res: null };
  agent_chat: { req: AgentChatRequest; res: AgentTurn };
  agent_decide: { req: AgentDecisionRequest; res: null };
  agent_cancel: { req: AgentRunRequest; res: null };
  agent_reset: { req: AgentChatIdRequest; res: null };
  agent_skills: { req: undefined; res: AgentSkill[] };
  detect_native_tools: { req: DetectToolsRequest; res: BackupSupport };
  backup_database: { req: BackupRequest; res: BackupReport };
  restore_database: { req: RestoreRequest; res: BackupReport };
  cancel_backup: { req: BackupRunRequest; res: null };
  cancel_query: { req: CancelQueryRequest; res: boolean };
  begin_transaction: { req: SessionRequest; res: boolean };
  commit_transaction: { req: SessionRequest; res: boolean };
  rollback_transaction: { req: SessionRequest; res: boolean };
  schema_diff: { req: SchemaDiffRequest; res: SchemaDiff };
  compare_table_data: { req: CompareTableDataRequest; res: DataCompare };
  cancel_workflow: { req: CancelWorkflowRequest; res: boolean };
  list_workflow_schedules: { req: undefined; res: WorkflowScheduleStatus[] };
  test_workflow_regex: { req: TestRegexRequest; res: RegexTestResult };
  workflow_last_node_config: { req: LastNodeConfigRequest; res: NodeConfig | null };
  list_diagram_customizations: { req: DiagramScopeRequest; res: DiagramCustomization[] };
  save_diagram_customizations: { req: SaveDiagramCustomizationsRequest; res: null };
  autosave_document: { req: AutosaveDocumentRequest; res: Document };
  checkpoint_document: { req: CheckpointDocumentRequest; res: DocumentVersionSummary };
  list_document_versions: { req: DocumentHistoryRequest; res: DocumentVersionSummary[] };
  restore_document_version: { req: DocumentVersionRequest; res: Document };
  delete_document_version: { req: DocumentVersionRequest; res: null };
  diagram_ddl: { req: DiagramDdlRequest; res: DiagramDdl };
  save_diagram_export: { req: SaveDiagramExportRequest; res: string };
  list_library: { req: undefined; res: LibraryLayout };
  create_library_folder: { req: CreateFolderRequest; res: LibraryFolder };
  rename_library_folder: { req: RenameRequest; res: LibraryFolder };
  move_library_folder: { req: MoveFolderRequest; res: LibraryFolder };
  delete_library_folder: { req: IdRequest; res: null };
  move_library_item: { req: MoveItemRequest; res: null };
  rename_document: { req: RenameRequest; res: Document };
  duplicate_document: { req: DuplicateDocumentRequest; res: Document };
  rename_saved_query: { req: RenameRequest; res: SavedQuery };
  duplicate_saved_query: { req: DuplicateQueryRequest; res: SavedQuery };
  list_saved_query_versions: { req: IdRequest; res: SavedQueryVersion[] };
  restore_saved_query_version: { req: IdRequest; res: SavedQuery };
  export_saved_queries: { req: ExportQueriesRequest; res: string };
  list_pins: { req: undefined; res: HomePin[] };
  pin_item: { req: HomeEntityRequest; res: null };
  unpin_item: { req: UnpinRequest; res: null };
  reorder_pins: { req: ReorderPinsRequest; res: null };
  list_recents: { req: undefined; res: HomeRecent[] };
  record_recent: { req: HomeEntityRequest; res: null };
  clear_recents: { req: undefined; res: null };
  rows_as_sql: { req: RowsSqlRequest; res: string };
  fetch_rows: { req: FetchRowsRequest; res: ResultSet };
  list_fk_remaps: { req: ConnectionIdRequest; res: FkRemap[] };
  save_fk_remap: { req: SaveFkRemapRequest; res: FkRemap };
  delete_fk_remap: { req: DeleteFkRemapRequest; res: null };
  ddl_profile: { req: CatalogRequest; res: DdlProfile };
  describe_table: { req: ColumnsRequest; res: TableSpec };
  preview_ddl: { req: PreviewDdlRequest; res: DdlPreview };
  list_table_tags: { req: ListTagsRequest; res: TagState };
  save_table_tag: { req: SaveTagRequest; res: TableTag };
  delete_table_tag: { req: IdRequest; res: null };
  reorder_table_tags: { req: ReorderTagsRequest; res: null };
  tag_table: { req: TagTableRequest; res: null };
  set_tag_collapsed: { req: TagCollapseRequest; res: null };
  move_tagged_table: { req: MoveTaggedTableRequest; res: null };
  // `execute_batch` also takes a progress channel: call it through `executeBatch`, not `ipc`.
  execute_batch: { req: ExecuteBatchRequest; res: BatchSummary };
  list_history_page: { req: HistoryPageRequest; res: HistoryPage };
  list_editor_history: { req: EditorHistoryRequest; res: HistoryEntry[] };
  read_sql_file: { req: ReadSqlFileRequest; res: OpenedSqlFile };
  scan_keys: { req: ScanKeysRequest; res: RedisKeyPage };
  key_info: { req: KeyInfoRequest; res: RedisKeyInfo };
  preview_export: { req: PreviewExportRequest; res: ExportPreview };
  preview_import: { req: PreviewImportRequest; res: ImportPreview };
  plan_json_import: { req: JsonPlanRequest; res: JsonImportPlan };
  run_json_import: { req: JsonImportRequest; res: JsonImportReport };
  inspect_sql_file: { req: SqlFileRequest; res: SqlFileInfo };
  import_sql_file: { req: SqlImportRequest; res: SqlImportReport };
  duplicate_connection: { req: ConnectionIdRequest; res: ConnectionSummary };
  validate_connection: { req: SaveConnectionRequest; res: ConnectionFieldIssue[] };
  credential_status: { req: undefined; res: CredentialStatus };
  reset_credentials: { req: undefined; res: CredentialStatus };
  list_connection_org: { req: undefined; res: ConnectionOrg };
  save_connection_tag: { req: SaveConnectionTagRequest; res: ConnectionTag };
  delete_connection_tag: { req: OrgIdRequest; res: null };
  save_connection_folder: { req: SaveConnectionFolderRequest; res: ConnectionFolder };
  delete_connection_folder: { req: OrgIdRequest; res: null };
  set_connection_org: { req: SetConnectionOrgRequest; res: ConnectionSummary };
  export_connections: { req: ExportConnectionsRequest; res: ConnectionExportReport };
  import_connections: { req: ImportConnectionsRequest; res: ConnectionImportReport };
  open_database_file: { req: OpenDatabaseFileRequest; res: ConnectionSummary };
  save_transient_connection: { req: SaveTransientRequest; res: ConnectionSummary };
  list_ssh_config_hosts: { req: undefined; res: SshConfigHost[] };
  create_demo_connection: { req: undefined; res: ConnectionSummary };
  open_window: { req: OpenWindowRequest; res: string };
  close_window: { req: undefined; res: null };
  set_ui_zoom: { req: SetZoomRequest; res: number };
  set_window_theme: { req: SetWindowThemeRequest; res: null };
  open_external: { req: OpenExternalRequest; res: null };
  open_log_folder: { req: undefined; res: null };
  log_ui_error: { req: LogUiErrorRequest; res: null };
  set_menu_shortcuts: { req: MenuShortcutsRequest; res: null };
  export_theme_file: { req: ExportThemeRequest; res: string };
  import_theme_file: { req: ImportThemeRequest; res: string };
  app_info: { req: undefined; res: AppInfo };
  ai_settings_save: { req: SaveAiSettingsRequest; res: AppSettings };
  ai_models: { req: AiModelsRequest; res: AiModelList };
  ai_test: { req: AiTestRequest; res: AiTestResult };
  ai_chat_list: { req: AiChatsRequest; res: AiChatSummary[] };
  ai_chat_load: { req: AgentChatIdRequest; res: AiChatDetail };
  ai_chat_rename: { req: AiChatRenameRequest; res: AiChatSummary };
  ai_chat_delete: { req: AgentChatIdRequest; res: null };
  ai_diagram_from_schema: { req: DiagramFromSchemaRequest; res: Document };
  close_session: { req: CloseSessionRequest; res: null };
}

type MissingFromMap = Exclude<CommandName, keyof CommandMap>;
type NotACommand = Exclude<keyof CommandMap, CommandName>;
export const commandMapIsExhaustive: [MissingFromMap, NotACommand] extends [never, never] ? true : never = true;

type Args<K extends CommandName> = CommandMap[K]["req"] extends undefined ? [] : [req: CommandMap[K]["req"]];

// WHAT:  Error class carrying the typed AppError, so `throw` always throws an Error.
export class IpcError extends Error {
  readonly error: AppError;
  constructor(error: AppError) {
    super(error.message);
    this.name = "IpcError";
    this.error = error;
  }
}

export async function ipc<K extends CommandName>(name: K, ...args: Args<K>): Promise<CommandMap[K]["res"]> {
  const req = args[0];
  try {
    return await invoke<CommandMap[K]["res"]>(name, req === undefined ? undefined : { req });
  } catch (raw) {
    throw new IpcError(normalizeError(raw));
  }
}

// WHAT:  The commands that report progress on a Tauri channel, and the frame each sends.
// WHY:   An export or an import runs for minutes; its reply is only the closing
//        report, so the dialog shows the frames as they arrive.
// HOW:   The Rust side names the channel parameter `on_progress`. Frames travel
//        apart from the reply and can land after it, so a caller treats a frame
//        that arrives once the call has returned as stale.
// WHERE: src-tauri/src/commands/transfer.rs
interface ProgressFrames {
  export_tables: ExportProgress;
  import_file: ImportProgress;
  import_sql_file: SqlImportProgress;
}

export async function ipcProgress<K extends keyof ProgressFrames>(name: K, req: CommandMap[K]["req"], onFrame: (frame: ProgressFrames[K]) => void): Promise<CommandMap[K]["res"]> {
  const channel = new Channel<ProgressFrames[K]>();
  channel.onmessage = onFrame;
  try {
    return await invoke<CommandMap[K]["res"]>(name, { req, onProgress: channel });
  } catch (raw) {
    throw new IpcError(normalizeError(raw));
  }
}

// WHAT:  Runs a script statement by statement and hands each progress event to
//        `onEvent` as it arrives: the plan, then started / finished per statement.
// WHY:   The result tabs fill in while the script is still running, which a single
//        reply cannot do. The events carry the outcomes, so the command's own
//        reply is only the closing summary and no row travels twice.
// HOW:   Channel messages travel separately from the command's reply, so the
//        reply can land first. The call therefore waits for the closing `done`
//        event as well, and gives up on it after a few seconds rather than hang.
// WHERE: src-tauri/src/commands/query.rs (execute_batch), src/stores/queryRuns.ts
export async function executeBatch(req: ExecuteBatchRequest, onEvent: (event: BatchEvent) => void): Promise<BatchSummary> {
  const channel = new Channel<BatchEvent>();
  let closed: () => void = () => undefined;
  const done = new Promise<void>((resolve) => {
    closed = resolve;
  });
  channel.onmessage = (event) => {
    onEvent(event);
    if (event.type === "done") closed();
  };
  try {
    const summary = await invoke<BatchSummary>("execute_batch", { req, onEvent: channel });
    let timer: ReturnType<typeof setTimeout> | undefined;
    const patience = new Promise<void>((resolve) => {
      timer = setTimeout(resolve, 5000);
    });
    await Promise.race([done, patience]);
    clearTimeout(timer);
    return summary;
  } catch (raw) {
    throw new IpcError(normalizeError(raw));
  }
}

// WHAT:  The only narrowing from `unknown` in the UI: Tauri rejects with the
//        serialised AppError, or with a string if the command itself was not found.
export function isAppError(raw: unknown): raw is AppError {
  return (
    typeof raw === "object" &&
    raw !== null &&
    "kind" in raw &&
    typeof raw.kind === "string" &&
    "message" in raw &&
    typeof raw.message === "string"
  );
}

export function normalizeError(raw: unknown): AppError {
  if (raw instanceof IpcError) return raw.error;
  if (isAppError(raw)) return raw;
  if (raw instanceof Error) return { kind: "internal", message: raw.message };
  if (typeof raw === "string") return { kind: "internal", message: raw };
  return { kind: "internal", message: JSON.stringify(raw) };
}

export function errorMessage(error: AppError): string {
  switch (error.kind) {
    case "destructive_confirmation_required":
      return `${error.message} ${error.statements.join("; ")}`;
    case "not_found":
    case "not_connected":
    case "read_only":
    case "invalid_input":
    case "timeout":
    case "cancelled":
    case "driver":
    case "store":
    case "crypto":
    case "keyring":
    case "internal":
      return error.message;
  }
}

// WHAT:  Download progress while an update installs.
// WHY:   Events are the other half of this boundary: the Rust side pushes bytes
//        as they arrive, and keeping the subscription here means components
//        never reach for @tauri-apps directly.
// WHERE: src-tauri/src/commands/updates.rs (emits "update:progress")
export function onUpdateProgress(handler: (progress: UpdateProgress) => void): Promise<UnlistenFn> {
  return listen<UpdateProgress>("update:progress", (event) => handler(event.payload));
}

// WHAT:  A connection's persisted schema cache was refreshed in the background and
//        the catalog differs from the copy `load_catalog` answered with at launch.
// WHERE: src-tauri/src/lib.rs (emits "schema-cache:updated"), src/stores/sessions.ts
export function onSchemaCacheUpdated(handler: (connectionId: string) => void): Promise<UnlistenFn> {
  return listen<SchemaCacheUpdated>("schema-cache:updated", (event) => handler(event.payload.connectionId));
}

// WHAT:  Everything one agent run emits, in order: prose tokens, tool calls,
//        permission prompts, charts it drew, and the finished turn.
// WHY:   A run takes many seconds over several tool calls, so the reply cannot
//        be one value — the chat renders it as it happens. Events carry a
//        `runId` so a listener can drop frames from a run it no longer shows.
// WHERE: src-tauri/src/commands/agent.rs (emits "agent:event")
export function onAgentEvent(handler: (event: AgentEvent) => void): Promise<UnlistenFn> {
  return listen<AgentEvent>("agent:event", (event) => handler(event.payload));
}

// WHAT:  Log lines and bytes-written while a native backup or restore runs.
// WHY:   pg_dump / mysqldump / mongodump report as they go; the dialog shows
//        that output live. Events carry a `runId` so a listener can drop frames
//        from a run it no longer shows.
// WHERE: src-tauri/src/commands/backup.rs (emits "backup:progress")
export function onBackupProgress(handler: (event: BackupEvent) => void): Promise<UnlistenFn> {
  return listen<BackupEvent>("backup:progress", (event) => handler(event.payload));
}

// WHAT:  Node-by-node status of a workflow run, whoever started it (the canvas, the
//        palette, or a Schedule node firing with the tab closed).
// WHY:   The engine runs in the core; the canvas only draws what it is told. Events
//        carry `workflowId` and `runId` so a listener can follow one run.
// WHERE: src-tauri/src/commands/workflows.rs (emits "workflow:event")
export function onWorkflowEvent(handler: (event: WorkflowEvent) => void): Promise<UnlistenFn> {
  return listen<WorkflowEvent>("workflow:event", (event) => handler(event.payload));
}
