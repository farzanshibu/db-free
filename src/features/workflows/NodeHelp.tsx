// SOT: workflow-node-help, template-syntax-help
import type { ReactNode } from "react";
import type { NodeKind } from "./model";

function C({ children }: { children: ReactNode }) {
  return <code className="font-mono text-foreground">{children}</code>;
}

const TEMPLATES: ReactNode = (
  <>
    <p>
      Text fields can use placeholders, filled from the rows that reach the node. <C>{"{{column}}"}</C> reads the first row; arithmetic works on numeric values (<C>{"{{i * 100}}"}</C>,{" "}
      <C>{"{{(a + b) / 2}}"}</C>); <C>{"{{date.now}}"}</C>, <C>{"{{date.today}}"}</C>, <C>{"{{date.time}}"}</C> and <C>{"{{date.unix}}"}</C> give the current UTC time.
    </p>
    <p>
      To include every row: <C>{"{{rows.table}}"}</C> (same as <C>{"{{rows}}"}</C>), <C>{"{{rows.json}}"}</C>, <C>{"{{rows.list}}"}</C>, <C>{"{{rows.html}}"}</C>, and the count <C>{"{{rows.count}}"}</C>. They list at most 50 rows and
      say how many more there are.
    </p>
  </>
);

const HELP: Record<NodeKind, ReactNode> = {
  schedule: (
    <>
      <p>
        A trigger. It fires the nodes after it on an interval (every N seconds, minutes, hours or days; never faster than every 10 seconds) or at a time of day on chosen weekdays in a chosen time zone. Daylight saving time is followed, and a
        time that does not exist on the day clocks jump forward fires right after the gap.
      </p>
      <p>
        Schedules fire only while <b>Schedules active</b> is on in the toolbar, even with the tab closed, as long as the app is running. A fire is skipped if the previous run of the workflow is still going. Each fire hands on one row,{" "}
        <C>{"{{firedAt}}"}</C>. Settings lists every schedule with its next fire.
      </p>
    </>
  ),
  query: (
    <>
      <p>Runs SQL on a connection (the workflow's own unless you pick another) and passes the rows on. A script's last result set is the output; a write passes on one row, <C>rowsAffected</C>.</p>
      {TEMPLATES}
      <p>Placeholders in SQL are written as safe literals for the connection, so a value can never become SQL. Read-only connections refuse writes, and destructive statements need this node's own opt-in.</p>
    </>
  ),
  http: (
    <>
      <p>Calls a URL and turns the JSON answer into rows. The response path (for example <C>data.results</C>) says where the rows are; an array gives one row per element and an object gives one row. A status outside 200 to 299 fails the node.</p>
      <p>The URL, the header values and the body accept placeholders from the first input row. Headers are a JSON object.</p>
    </>
  ),
  read_file: (
    <>
      <p>Loads rows from CSV, TSV, JSON or NDJSON. CSV / TSV use the first row as the header and read quoted fields with embedded delimiters, quotes and line breaks; an empty field is NULL. The node has no input and reads the file again on every run.</p>
    </>
  ),
  filter: (
    <>
      <p>Keeps the rows that match all (or any) of the rules. An empty rule list passes every row. Numeric operators compare numbers; the others compare text; "is NULL" means SQL NULL, not an empty string.</p>
    </>
  ),
  row_count_gate: (
    <>
      <p>Lets the rows through only while the row count satisfies the rule. When it does not, the branch stops here: everything after the gate is skipped, so an alert only goes out when a query actually found something.</p>
    </>
  ),
  branch: (
    <>
      <p>Splits the rows: matches leave by the True handle, the rest by False. Wire each side by dragging from its handle. A side with no rows is skipped along with everything after it.</p>
    </>
  ),
  repeat: (
    <>
      <p>
        Runs everything after it once per iteration, and each pass carries <C>{"{{i}}"}</C> (from 0) and <C>{"{{total}}"}</C>. Page through a table with <C>{"OFFSET {{i * 100}}"}</C> or an API with <C>{"?page={{i + 1}}"}</C>. Passes
        run one after another and stop at the first failure.
      </p>
    </>
  ),
  delay: (
    <>
      <p>Waits, then passes the rows on unchanged. Handy for spacing out calls to a rate-limited API. Stop ends the wait at once.</p>
    </>
  ),
  regex: (
    <>
      <p>
        Per-row regex on one column. Extract writes the first capture group (or the whole match) to the result column, NULL when nothing matches. Replace substitutes matches, using <C>$1</C>, <C>$2</C> for groups and <C>$&</C> for the whole
        match. Flags: i ignore case, m multi-line, s dot matches newline, g all matches (Replace changes only the first match without it).
      </p>
      <p>The tester uses the same engine as the run. The syntax is Rust's: no look-around and no back-references.</p>
    </>
  ),
  javascript: (
    <>
      <p>
        Runs your code as the body of an async function with <C>rows</C>, <C>fields</C> and <C>ctx.aborted</C>. Return an array of rows, <C>{"{ rows, fields }"}</C>, or nothing to pass the input through. The sandbox has no network, no
        files and no process access; a script that runs too long, uses too much memory or is stopped ends with an error. <C>console.log</C> output appears in the Output tab.
      </p>
    </>
  ),
  save_file: (
    <>
      <p>Writes the rows to a file: CSV, TSV, JSON, NDJSON, a Markdown table or SQL INSERT statements. Leave the path blank and the first manual run asks where to save it; the choice is remembered. The file is replaced on every run.</p>
    </>
  ),
  insert: (
    <>
      <p>
        Inserts the rows into an existing table, one at a time, matching fields to columns by name (extra fields are skipped). The first row that fails stops the node and is named ("Row 12"). Writes follow the connection's read-only lock.
      </p>
    </>
  ),
  notification: (
    <>
      <p>Shows a native desktop notification with a templated title and body.</p>
      {TEMPLATES}
    </>
  ),
  email: (
    <>
      <p>Sends one email per run through your SMTP server. To, subject and body accept placeholders; a body with an HTML tag is sent as HTML. The password is stored encrypted and never shown again; a new Email node starts from the last setup you saved.</p>
      {TEMPLATES}
    </>
  ),
  slack: (
    <>
      <p>Posts a message to a Slack incoming webhook. The webhook URL is a secret: it is stored encrypted and never shown again; a new Slack node starts from the last webhook you saved.</p>
      {TEMPLATES}
    </>
  ),
  note: (
    <>
      <p>A comment on the canvas. Notes are never run and never connect to anything.</p>
    </>
  ),
};

export function NodeHelp({ kind }: { kind: NodeKind }) {
  return <div className="flex flex-col gap-2 text-xs leading-relaxed text-muted">{HELP[kind]}</div>;
}
