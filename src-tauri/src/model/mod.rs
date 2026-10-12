// SOT: model-index, shared-shapes

pub mod agent;
pub mod backup;
pub mod changes;
pub mod compare;
pub mod connection;
pub mod documents;
pub mod objects;
pub mod query;
pub mod schema;
pub mod settings;
pub mod transfer;
pub mod update;
pub mod value;
pub mod workflow;
pub mod diagrams;
pub mod library;
pub mod grid;
pub mod ddl;
pub mod tags;
pub mod editor;
pub mod keys;
pub mod connection_org;
pub mod auth;
pub mod shell;
pub mod engine_events;

pub use agent::{
    AgentArtifact, AgentAutonomy, AgentEvent, AgentSkill, AgentStop, AgentTurn, AgentUsage,
    PermissionDecision, PermissionRequest, ToolCallRecord, ToolStatus, UiBlock, UiGraphEdge,
    UiGraphNode, UiStat, UiTone,
};
pub use backup::{
    BackupEvent, BackupFormat, BackupMethod, BackupOptions, BackupReport, BackupSupport, NativeTool, NativeToolStatus,
};
pub use connection::{
    ConnectionColor, ConnectionInput, ConnectionRecord, ConnectionSummary, Engine, EngineFacts, EngineKind, Environment, Family,
    FormKind, ResolvedConnection, SshAuth, SshTunnel, SslMode,
};
pub use changes::{CellValue, ChangePreview, StagedChange};
pub use compare::{
    ColumnChange, ColumnDiff, CompareDirection, DataCompare, DiffStatus, ForeignKeyDiff, RowDiff, RowStatus, SchemaDiff, TableDiff,
};
pub use documents::{
    ConditionOp, DashboardBody, DashboardVariable, DiagramBody, DiagramColumn, DiagramRelation, DiagramTable,
    Document, DocumentBody, DocumentKind, Widget, WidgetCondition, WidgetKind, WorkflowBody, WorkflowRunReport,
    WorkflowStep, WorkflowStepResult,
};
pub use objects::{
    CodeLanguage, FacetCounts, FacetValue, ObjectAction, ObjectDetail, ObjectKind, ObjectProperty, ObjectRef, ObjectSummary,
    RangeQueryRequest, RangeResult, SearchRequest, SearchResult, Series, ServerStats, Stat, StatGroup, Tool,
    VectorSearchRequest,
};
pub use query::{
    ColumnMeta, EditorBuffer, FilterOp, FilterRule, HistoryEntry, HistoryOrigin, HistoryStatus,
    PageQuery, QueryOutcome, ResultSet, SavedQuery, SortRule, StatementIntent, StatementResult,
    StatementSpan, TablePage,
};
pub use settings::{AiProvider, AiSettings, AppSettings, ExecutionMode, RunScope};
pub use transfer::{AiReply, ExportReport, ExportedFile, ImportReport, PlanNode, PlanReport, TransferFormat};
pub use schema::{ColumnInfo, ForeignKey, SchemaCatalog, SchemaInfo, TableInfo, TableKind, TableRef};
pub use value::Value;
pub use update::{UpdateProgress, UpdateStatus};
pub use workflow::{
    NodeConfig, NodeRunState, NodeSecret, NodeUpdate, RegexTestResult, RunStatus, RunTrigger, ScheduleMode, WorkflowEdge,
    WorkflowEvent, WorkflowNode, WorkflowScheduleStatus,
};
pub use documents::{LegendMode, MetricFormat, ValueFormat};
pub use diagrams::{
    DiagramColumnPair, DiagramCustom, DiagramCustomKind, DiagramCustomization, DiagramCustomizationKey, DiagramDdl, DiagramExportFormat,
    DiagramLineWidth, DiagramNote, DiagramPoint, DiagramSide, DocumentVersionSummary, ReferentialAction,
};
pub use library::{
    HomeEntity, HomeItemKind, HomePin, HomeRecent, LibraryFolder, LibraryLayout, LibraryPlacement, LibraryScope, SavedQueryVersion,
};
pub use changes::{ChangeResult, CommitReport, ReturnedRows, RowSqlKind, SqlToken};
pub use grid::FkRemap;
pub use query::{ColumnTraits, FilterMode, TruncatedCell};
pub use ddl::{
    AlterTableSpec, ColumnDefault, ColumnSpec, DdlOperation, DdlPreview, DdlProfile, DdlSupport, DdlType, DuplicateSpec,
    FkAction, ForeignKeySpec, IndexSpec, NamespaceCaps, NamespaceKind, NamespaceOp, NamespaceSpec, RemoveAction, RemoveSpec, TableSpec,
    TypeParams,
};
pub use tags::{TableTag, TableTagInput, TableTagLink, TagState, MAX_TAG_NAME_LEN};
pub use editor::{BatchEvent, BatchStatementPlan, BatchStatementResult, BatchStatus, BatchSummary, HistoryPage, OpenedSqlFile};
pub use keys::{RedisKeyEntry, RedisKeyInfo, RedisKeyPage, RedisKeyScan, RedisKeyType};
pub use transfer::{
    CsvImportOptions, ExportFormat, ExportJsonShape, ExportLayout, ExportOptions, ExportPreview, ExportProgress, ExportStage,
    ImportColumn, ImportConflict, ImportConflictSupport, ImportMapping, ImportOperation, ImportOperationChoice,
    ImportOperationKind, ImportOperationResult, ImportPreview, ImportProgress, JsonFileShape, JsonImportPlan,
    JsonImportReport, SqlFileInfo, SqlImportProgress, SqlImportReport,
};
pub use connection_org::{
    ConnectionExportReport, ConnectionExtras, ConnectionField, ConnectionFieldIssue, ConnectionFolder, ConnectionImportReport,
    ConnectionImportSource, ConnectionOpenedEvent, ConnectionOrg, ConnectionSkipped, ConnectionTag, ConnectionTagColor, CredentialStatus, KeyStorage,
    SshConfigHost, TlsFiles,
};
pub use auth::{
    AuthSecretFlags, AuthSecrets, AwsAccess, AwsCredentialSource, AzureAdMode, ConnectionAuth, GcpAuthMode, OidcProvider,
};
pub use shell::{AppErrorEvent, AppInfo, CustomTheme, MenuActionEvent, ThemeBase};
pub use agent::{
    AiChatDetail, AiChatMessage, AiChatRole, AiChatSummary, AiDataAccess, AiKeyChange, AiModel, AiModelList,
    AiProfile, AiTestResult, ChatMention, MentionKind, ModelChoice,
};
pub use engine_events::SchemaCacheUpdated;
