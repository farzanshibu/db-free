# Security Policy

## Supported versions

DB Free is pre-1.0 and moves fast. Only the latest `main` and the latest
tagged release receive security fixes. If you are on an older installer,
please update to the newest GitHub release first and check whether the issue
still reproduces.

| Version        | Supported          |
| -------------- | ------------------ |
| latest release | :white_check_mark: |
| latest `main`  | :white_check_mark: |
| older releases | :x:                |

## Reporting a vulnerability

**Do not open a public issue, PR, or discussion for a suspected
vulnerability.** Secrets handling (AES-256-GCM + OS keychain), connection
trust, and the auto-update feed make private disclosure important here.

Email **farzanshibu786@gmail.com** with:

* A description of the vulnerability and its impact
* Steps to reproduce (minimal app + engine setup, queries, or config)
* The DB Free version/commit, OS + arch, and engine/server version
* Any logs (`RUST_LOG=debug`) with secrets redacted

What happens next:

1. Acknowledgement within 72 hours.
2. We investigate, confirm severity, and agree on a fix + disclosure timeline
   with you (we aim to ship a fix within 90 days for confirmed issues).
3. We publish a fixed release and credit you unless you prefer to stay
   anonymous. Please avoid public disclosure until the fix is released.

## Scope notes

* Connection secrets are sealed with AES-256-GCM with the key in the OS
  keychain and are never returned to the UI (`ConnectionSummary.hasSecret`
  only). Reports showing secret leakage are treated as high severity.
* The in-app updater verifies bundles against the signed `latest.json`
  published with each GitHub release — please report any update-integrity
  concern immediately.
* Cloud-only engines (Snowflake, BigQuery, Firestore, Pinecone, …) need real
  credentials for live testing; do not include live credentials in any report.
  Redact connection strings, tokens, and keys from everything you send.

Thanks for helping keep DB Free and its users safe.
