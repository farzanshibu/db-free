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
// @parity wf
// @end wf
// @parity dash
// @end dash
// @parity diag
// @end diag
// @parity lib
// @end lib
// @parity grid
// @end grid
// @parity schema
// @end schema
// @parity editor
// @end editor
// @parity rx
// @end rx
// @parity conn
// @end conn
// @parity auth
// @end auth
// @parity shell
// @end shell
// @parity ai
// @end ai
// @parity engine
// @end engine
