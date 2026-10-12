// SOT: services-index, orchestration-layer
//
// WHAT:  Services orchestrate store + integrations for one use case each.
// WHY:   Commands stay thin (validate, guard, call); integrations stay dumb.
// WHERE: scripts/guardrail.py — commands may not import store/integrations directly.

pub mod agent;
pub mod ai;
pub mod backup;
pub mod buffers;
pub mod changes;
pub mod connection;
pub mod data;
pub mod data_compare;
pub mod documents;
pub mod history;
pub mod objects;
pub mod query;
pub mod saved_queries;
pub mod scripts;
pub mod schema;
pub mod schema_diff;
pub mod settings;
pub mod transfer;
pub mod workflows;
pub mod diagram_ddl;
pub mod diagrams;
pub mod document_versions;
pub mod home;
pub mod library;
pub mod fk_remaps;
pub mod tags;
pub mod shell_commands;
pub mod export;
pub mod keys;
pub mod sql_import;
pub mod connection_files;
pub mod connection_formats;
pub mod connection_org;
pub mod connection_transfer;
pub mod theme_files;
pub mod ai_chats;
pub mod ai_profiles;
pub mod schema_cache;
pub mod sessions;
