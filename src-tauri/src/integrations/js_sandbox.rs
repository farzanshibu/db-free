// SOT: javascript-sandbox, quickjs-adapter, workflow-javascript-node, script-interrupt, script-console-capture

use rquickjs::{CatchResultExt, CaughtError, Context, Ctx, Function, Object, Promise, Runtime, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

// ============================================================================
// JAVASCRIPT SANDBOX FOR THE WORKFLOW JAVASCRIPT NODE
//
// WHAT:  Runs a user's script over a set of rows and hands back the rows it
//        returns, plus whatever it wrote to `console`.
// WHY:   The JavaScript node is the escape hatch of a workflow. It must not be
//        an escape hatch for the machine: no files, no network, no process, and
//        it must stop when the user presses Stop even in the middle of
//        `while (true) {}`.
// HOW:   QuickJS, a JavaScript engine with no host APIs of its own, so there is
//        no `fetch`, `require`, `process` or file access to remove. On top of
//        that: an interrupt handler (Stop and a wall-clock limit), a memory
//        ceiling and a stack ceiling. Rows go in and out as JSON text, so no
//        engine type crosses this module's boundary. `setTimeout` and friends
//        are a small polyfill driven from here, so `await new Promise(r =>
//        setTimeout(r, 100))` works for pacing. The script is the body of an
//        async function that receives `rows`, `fields` and `ctx` (`ctx.aborted`
//        turns true after Stop); it may return an array of rows, `{ rows, fields }`,
//        or nothing (the input passes through).
// WHERE: src-tauri/src/services/workflows/nodes/transform.rs
// ============================================================================

/// How long a script may run, timers included.
pub const SCRIPT_TIME_LIMIT: Duration = Duration::from_secs(300);
const MEMORY_LIMIT: usize = 256 * 1024 * 1024;
const STACK_LIMIT: usize = 512 * 1024;
/// The longest the driver sleeps between looks at the Stop flag.
const POLL: Duration = Duration::from_millis(20);

#[derive(Debug, Clone)]
pub struct ScriptRequest {
    pub code: String,
    /// The input rows as a JSON array of objects.
    pub rows_json: String,
    /// The input columns as a JSON array of `{ name, dataType }`.
    pub fields_json: String,
    pub time_limit: Duration,
}

/// What the script handed back.
#[derive(Debug, Clone, PartialEq)]
pub enum ScriptOutcome {
    /// It returned nothing: the input stands.
    Passthrough,
    Rows { rows: Vec<serde_json::Value>, fields: Vec<String> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptErrorKind {
    Syntax,
    Runtime,
    Stopped,
    TimedOut,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptError {
    pub kind: ScriptErrorKind,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct ConsoleEntry {
    pub level: String,
    pub text: String,
}

#[derive(Debug)]
pub struct ScriptReport {
    pub logs: Vec<ConsoleEntry>,
    pub result: Result<ScriptOutcome, ScriptError>,
}

// WHAT:  Runs a script on a blocking thread and stops it when `cancel` fires.
pub async fn run(request: ScriptRequest, cancel: CancellationToken) -> ScriptReport {
    let abort = Arc::new(AtomicBool::new(false));
    let worker_abort = Arc::clone(&abort);
    let mut handle = tokio::task::spawn_blocking(move || run_blocking(&request, &worker_abort));
    let joined = tokio::select! {
        joined = &mut handle => joined,
        () = cancel.cancelled() => {
            abort.store(true, Ordering::SeqCst);
            handle.await
        }
    };
    joined.unwrap_or_else(|_| ScriptReport {
        logs: Vec::new(),
        result: Err(ScriptError { kind: ScriptErrorKind::Runtime, message: "The script engine stopped unexpectedly.".to_string() }),
    })
}

/// The wrapper the user's code is placed in. `__host` carries `log` and `aborted`.
const PROLOGUE: &str = r##"(function (__host, __rowsJson, __fieldsJson) {
"use strict";
const __text = (v) => {
  if (v === undefined) return "undefined";
  if (v === null) return "null";
  if (typeof v === "string") return v;
  if (typeof v === "number" || typeof v === "boolean" || typeof v === "bigint") return String(v);
  if (typeof v === "function") return v.toString();
  try { return JSON.stringify(v, null, 2); } catch (e) { try { return String(v); } catch (e2) { return "[unserialisable]"; } }
};
const __logger = (level) => (...args) => { __host.log(level, args.map(__text).join(" ")); };
const console = { log: __logger("log"), info: __logger("info"), warn: __logger("warn"), error: __logger("error"), debug: __logger("debug") };
const __timers = [];
let __nextTimer = 1;
const setTimeout = (fn, ms, ...args) => { const id = __nextTimer++; __timers.push({ id, at: Date.now() + Math.max(0, Number(ms) || 0), fn, args, every: 0 }); return id; };
const setInterval = (fn, ms, ...args) => { const id = __nextTimer++; const every = Math.max(1, Number(ms) || 0); __timers.push({ id, at: Date.now() + every, fn, args, every }); return id; };
const __clear = (id) => { const i = __timers.findIndex((t) => t.id === id); if (i >= 0) __timers.splice(i, 1); };
const clearTimeout = __clear;
const clearInterval = __clear;
const __pump = () => {
  if (__timers.length === 0) return -1;
  __timers.sort((a, b) => a.at - b.at);
  const t = __timers[0];
  const wait = t.at - Date.now();
  if (wait > 0) return wait;
  __timers.shift();
  if (t.every) { t.at = Date.now() + t.every; __timers.push(t); }
  t.fn(...t.args);
  return 0;
};
const rows = JSON.parse(__rowsJson);
const fields = JSON.parse(__fieldsJson);
const ctx = {};
Object.defineProperty(ctx, "aborted", { get: () => __host.aborted(), enumerable: true });
const __user = async function (rows, fields, ctx) {
"##;

const EPILOGUE: &str = r##"
};
const __shape = (r) => {
  if (r === undefined || r === null) return JSON.stringify({ kind: "passthrough" });
  let outRows;
  let outFields = null;
  if (Array.isArray(r)) outRows = r;
  else if (typeof r === "object" && Array.isArray(r.rows)) { outRows = r.rows; if (Array.isArray(r.fields)) outFields = r.fields; }
  else throw new Error("Code must return an array of rows, an object with a `rows` array, or nothing.");
  let names;
  if (outFields) names = outFields.map((f) => (typeof f === "string" ? f : f && f.name)).filter((n) => typeof n === "string");
  else {
    names = [];
    const seen = new Set();
    for (const row of outRows) {
      if (row && typeof row === "object" && !Array.isArray(row)) {
        for (const k of Object.keys(row)) { if (!seen.has(k)) { seen.add(k); names.push(k); } }
      }
    }
  }
  return JSON.stringify({ kind: "rows", rows: outRows, fields: names });
};
return { run: async () => __shape(await __user(rows, fields, ctx)), pump: __pump };
})"##;

fn run_blocking(request: &ScriptRequest, abort: &Arc<AtomicBool>) -> ScriptReport {
    let logs: Arc<Mutex<Vec<ConsoleEntry>>> = Arc::new(Mutex::new(Vec::new()));
    let outcome = execute(request, abort, &logs);
    let logs = logs.lock().map(|l| l.clone()).unwrap_or_default();
    let result = outcome.map_err(|mut err| {
        // An interrupt surfaces as a generic engine error; say what really happened.
        if abort.load(Ordering::SeqCst) {
            err = ScriptError { kind: ScriptErrorKind::Stopped, message: "Stopped".to_string() };
        }
        err
    });
    ScriptReport { logs, result }
}

fn execute(request: &ScriptRequest, abort: &Arc<AtomicBool>, logs: &Arc<Mutex<Vec<ConsoleEntry>>>) -> Result<ScriptOutcome, ScriptError> {
    let runtime = Runtime::new().map_err(engine_failure)?;
    runtime.set_memory_limit(MEMORY_LIMIT);
    runtime.set_max_stack_size(STACK_LIMIT);
    let started = Instant::now();
    let limit = request.time_limit;
    let interrupt_abort = Arc::clone(abort);
    runtime.set_interrupt_handler(Some(Box::new(move || interrupt_abort.load(Ordering::SeqCst) || started.elapsed() > limit)));
    let context = Context::full(&runtime).map_err(engine_failure)?;
    context.with(|ctx| drive(&ctx, request, abort, logs, started))
}

fn engine_failure(err: rquickjs::Error) -> ScriptError {
    ScriptError { kind: ScriptErrorKind::Runtime, message: format!("The script engine could not start: {err}") }
}

fn drive(
    ctx: &Ctx<'_>,
    request: &ScriptRequest,
    abort: &Arc<AtomicBool>,
    logs: &Arc<Mutex<Vec<ConsoleEntry>>>,
    started: Instant,
) -> Result<ScriptOutcome, ScriptError> {
    let source = format!("{PROLOGUE}{}\n{EPILOGUE}", request.code);
    // Compiling the wrapper is where a syntax error in the user's code shows up.
    let factory: Function = ctx.eval(source).catch(ctx).map_err(|e| failure(&e, ScriptErrorKind::Syntax))?;
    let host = build_host(ctx, abort, logs).catch(ctx).map_err(|e| failure(&e, ScriptErrorKind::Runtime))?;
    let api: Object = factory
        .call((host, request.rows_json.clone(), request.fields_json.clone()))
        .catch(ctx)
        .map_err(|e| failure(&e, ScriptErrorKind::Runtime))?;
    let run: Function = api.get("run").catch(ctx).map_err(|e| failure(&e, ScriptErrorKind::Runtime))?;
    let pump: Function = api.get("pump").catch(ctx).map_err(|e| failure(&e, ScriptErrorKind::Runtime))?;
    let promise: Promise = run.call(()).catch(ctx).map_err(|e| failure(&e, ScriptErrorKind::Runtime))?;

    loop {
        while ctx.execute_pending_job() {}
        if promise.state() != rquickjs::promise::PromiseState::Pending {
            break;
        }
        if abort.load(Ordering::SeqCst) {
            return Err(ScriptError { kind: ScriptErrorKind::Stopped, message: "Stopped".to_string() });
        }
        if started.elapsed() > request.time_limit {
            return Err(ScriptError { kind: ScriptErrorKind::TimedOut, message: "The script ran for too long and was stopped.".to_string() });
        }
        let wait: f64 = pump.call(()).catch(ctx).map_err(|e| failure(&e, ScriptErrorKind::Runtime))?;
        if wait < 0.0 {
            return Err(ScriptError {
                kind: ScriptErrorKind::Runtime,
                message: "The script is waiting for something that will never happen (an unresolved promise).".to_string(),
            });
        }
        if wait > 0.0 || wait.is_nan() {
            let pause = if wait.is_finite() { Duration::from_secs_f64((wait / 1000.0).clamp(0.0, 1.0)).min(POLL) } else { POLL };
            std::thread::sleep(pause);
        }
    }
    let json: String = match promise.result::<String>() {
        Some(result) => result.catch(ctx).map_err(|e| failure(&e, ScriptErrorKind::Runtime))?,
        None => return Err(ScriptError { kind: ScriptErrorKind::Runtime, message: "The script did not finish.".to_string() }),
    };
    parse_outcome(&json)
}

fn build_host<'js>(ctx: &Ctx<'js>, abort: &Arc<AtomicBool>, logs: &Arc<Mutex<Vec<ConsoleEntry>>>) -> rquickjs::Result<Object<'js>> {
    let host = Object::new(ctx.clone())?;
    let sink = Arc::clone(logs);
    host.set(
        "log",
        Function::new(ctx.clone(), move |level: String, text: String| {
            if let Ok(mut lines) = sink.lock() {
                if lines.len() < 2_000 {
                    lines.push(ConsoleEntry { level, text });
                }
            }
        })?,
    )?;
    let flag = Arc::clone(abort);
    host.set("aborted", Function::new(ctx.clone(), move || flag.load(Ordering::SeqCst))?)?;
    Ok(host)
}

fn parse_outcome(json: &str) -> Result<ScriptOutcome, ScriptError> {
    let parsed: serde_json::Value = serde_json::from_str(json)
        .map_err(|e| ScriptError { kind: ScriptErrorKind::Runtime, message: format!("The script returned something that is not JSON: {e}") })?;
    if parsed.get("kind").and_then(serde_json::Value::as_str) != Some("rows") {
        return Ok(ScriptOutcome::Passthrough);
    }
    let rows = parsed.get("rows").and_then(serde_json::Value::as_array).cloned().unwrap_or_default();
    let fields = parsed
        .get("fields")
        .and_then(serde_json::Value::as_array)
        .map(|names| names.iter().filter_map(|n| n.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    Ok(ScriptOutcome::Rows { rows, fields })
}

fn failure(err: &CaughtError<'_>, kind: ScriptErrorKind) -> ScriptError {
    let message = match err {
        CaughtError::Exception(exception) => {
            let message = exception.message().unwrap_or_default();
            let name: Option<String> = exception.as_object().get("name").ok();
            match name {
                Some(name) if !name.is_empty() && name != "Error" && !message.starts_with(&name) => format!("{name}: {message}"),
                _ => message,
            }
        }
        CaughtError::Value(value) => describe_value(value),
        CaughtError::Error(other) => other.to_string(),
    };
    // A script that throws while it is being compiled is a syntax problem; one that
    // fails to start for another reason is not.
    let kind = if kind == ScriptErrorKind::Syntax && !message.contains("SyntaxError") { ScriptErrorKind::Runtime } else { kind };
    ScriptError { kind, message }
}

fn describe_value(value: &Value<'_>) -> String {
    value.as_string().and_then(|s| s.to_string().ok()).unwrap_or_else(|| "The script threw a value that is not an error.".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(code: &str, rows: &str) -> ScriptRequest {
        ScriptRequest {
            code: code.to_string(),
            rows_json: rows.to_string(),
            fields_json: r#"[{"name":"n","dataType":"integer"}]"#.to_string(),
            time_limit: Duration::from_secs(20),
        }
    }

    async fn go(code: &str, rows: &str) -> ScriptReport {
        run(request(code, rows), CancellationToken::new()).await
    }

    #[tokio::test]
    async fn returned_rows_come_back_with_field_order() {
        let report = go("return rows.map(r => ({ b: r.n * 2, a: r.n }));", r#"[{"n":1},{"n":2}]"#).await;
        match report.result {
            Ok(ScriptOutcome::Rows { rows, fields }) => {
                assert_eq!(fields, vec!["b", "a"], "the script's own key order is kept");
                assert_eq!(rows[1]["b"], serde_json::json!(4));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[tokio::test]
    async fn rows_object_with_fields_and_passthrough() {
        let report = go("return { rows: [{ x: 1 }], fields: [{ name: 'x' }, 'y'] };", "[]").await;
        assert!(matches!(report.result, Ok(ScriptOutcome::Rows { ref fields, .. }) if fields == &vec!["x".to_string(), "y".to_string()]));
        let nothing = go("const a = 1;", "[]").await;
        assert_eq!(nothing.result.unwrap(), ScriptOutcome::Passthrough);
        let bad = go("return 42;", "[]").await;
        assert!(matches!(bad.result, Err(ScriptError { kind: ScriptErrorKind::Runtime, .. })));
    }

    #[tokio::test]
    async fn console_output_is_captured() {
        let report = go("console.log('hi', 1, {a: 1}); console.warn('careful'); return rows;", "[]").await;
        assert_eq!(report.logs.len(), 2);
        assert_eq!(report.logs[0].level, "log");
        assert_eq!(report.logs[0].text, "hi 1 {\n  \"a\": 1\n}");
        assert_eq!(report.logs[1].level, "warn");
        assert!(report.result.is_ok());
    }

    #[tokio::test]
    async fn syntax_and_runtime_errors_are_told_apart() {
        let syntax = go("return rows.map(;", "[]").await;
        assert!(matches!(syntax.result, Err(ScriptError { kind: ScriptErrorKind::Syntax, .. })), "{:?}", syntax.result);
        let runtime = go("return nothing.here;", "[]").await;
        match runtime.result {
            Err(ScriptError { kind: ScriptErrorKind::Runtime, message }) => assert!(message.contains("nothing"), "{message}"),
            other => panic!("unexpected {other:?}"),
        }
        let thrown = go("throw new Error('boom');", "[]").await;
        assert!(matches!(thrown.result, Err(ScriptError { ref message, .. }) if message.contains("boom")));
    }

    #[tokio::test]
    async fn there_is_no_host_access() {
        let report = go(
            "return [{ fetch: typeof fetch, require: typeof require, process: typeof process, std: typeof std, os: typeof os, xhr: typeof XMLHttpRequest }];",
            "[]",
        )
        .await;
        let Ok(ScriptOutcome::Rows { rows, .. }) = report.result else { panic!("expected rows") };
        for key in ["fetch", "require", "process", "std", "os", "xhr"] {
            assert_eq!(rows[0][key], serde_json::json!("undefined"), "{key} must not exist");
        }
    }

    #[tokio::test]
    async fn timers_and_awaits_work() {
        let report = go("await new Promise(r => setTimeout(r, 30)); const t = Date.now(); await 0; return [{ ok: t > 0 }];", "[]").await;
        assert!(report.result.is_ok(), "{:?}", report.result);
        let stuck = go("await new Promise(() => {}); return rows;", "[]").await;
        assert!(matches!(stuck.result, Err(ScriptError { kind: ScriptErrorKind::Runtime, .. })));
    }

    #[tokio::test]
    async fn stop_interrupts_a_busy_loop() {
        let cancel = CancellationToken::new();
        let token = cancel.clone();
        let stopper = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(150)).await;
            token.cancel();
        });
        let started = Instant::now();
        let report = run(request("while (true) {}", "[]"), cancel).await;
        stopper.await.unwrap();
        assert!(matches!(report.result, Err(ScriptError { kind: ScriptErrorKind::Stopped, .. })), "{:?}", report.result);
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[tokio::test]
    async fn ctx_aborted_turns_true_after_stop_and_the_time_limit_applies() {
        let mut limited = request("while (true) {}", "[]");
        limited.time_limit = Duration::from_millis(200);
        let report = run(limited, CancellationToken::new()).await;
        assert!(report.result.is_err());
        let report = go("return [{ aborted: ctx.aborted }];", "[]").await;
        let Ok(ScriptOutcome::Rows { rows, .. }) = report.result else { panic!("expected rows") };
        assert_eq!(rows[0]["aborted"], serde_json::json!(false));
    }

    #[tokio::test]
    async fn memory_is_capped() {
        let report = go("const a = []; while (true) { a.push(new Array(1000000).fill(1)); }", "[]").await;
        assert!(report.result.is_err(), "a runaway allocation must end in an error, not an abort");
    }
}
