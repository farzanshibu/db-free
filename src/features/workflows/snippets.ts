// SOT: workflow-javascript-snippets, workflow-email-templates
// WHAT:  Starting points offered in the JavaScript and Email node forms.
// WHY:   Both nodes are blank pages; a handful of working examples teaches the shape
//        of the code (`rows`, `fields`, `ctx.aborted`) and of the placeholders faster
//        than documentation.
// WHERE: src/features/workflows/forms/TransformForms.tsx, src/features/workflows/forms/NotifyForms.tsx

export interface Snippet {
  label: string;
  description: string;
  code: string;
}

export const JS_SNIPPETS: readonly Snippet[] = [
  {
    label: "Add a column",
    description: "Append a computed field to every row.",
    code: `// Add a "fullName" column derived from existing fields.
return rows.map(r => ({
  ...r,
  fullName: \`\${r.firstName ?? ""} \${r.lastName ?? ""}\`.trim(),
}));`,
  },
  {
    label: "Filter rows",
    description: "Drop rows that do not match a condition.",
    code: `// Keep only rows where amount > 100.
return rows.filter(r => Number(r.amount) > 100);`,
  },
  {
    label: "Sort rows",
    description: "Reorder rows by a column.",
    code: `// Sort by createdAt, newest first.
return [...rows].sort((a, b) =>
  String(b.createdAt).localeCompare(String(a.createdAt))
);`,
  },
  {
    label: "Take first N",
    description: "Keep only the first N rows.",
    code: `// Keep the first 10 rows.
return rows.slice(0, 10);`,
  },
  {
    label: "Pick columns",
    description: "Project a narrower row shape.",
    code: `// Output only id and name.
return rows.map(({ id, name }) => ({ id, name }));`,
  },
  {
    label: "Aggregate to one row",
    description: "Collapse the rows to a single summary row.",
    code: `// Count and sum across all rows.
const total = rows.reduce((sum, r) => sum + Number(r.amount || 0), 0);
return [{ count: rows.length, total }];`,
  },
  {
    label: "Group by",
    description: "Bucket rows by a key, then summarise each bucket.",
    code: `// Count rows per category.
const buckets = {};
for (const r of rows) {
  const key = r.category ?? "(none)";
  (buckets[key] ??= []).push(r);
}
return Object.entries(buckets).map(([key, items]) => ({ key, count: items.length }));`,
  },
  {
    label: "Pace work with await",
    description: "Pause between rows; stop cleanly on Stop.",
    code: `// Process one row at a time, pausing between rows.
const out = [];
for (const r of rows) {
  if (ctx.aborted) break;
  await new Promise(resolve => setTimeout(resolve, 100));
  out.push({ ...r, doubled: Number(r.value) * 2 });
}
return out;`,
  },
  {
    label: "Pass through",
    description: "Return the input unchanged.",
    code: "return rows;",
  },
];

export interface EmailTemplate {
  label: string;
  description: string;
  subject: string;
  body: string;
}

export const EMAIL_TEMPLATES: readonly EmailTemplate[] = [
  {
    label: "Plain greeting",
    description: "A short plain-text note to one person.",
    subject: "Hi {{name}}",
    body: `Hi {{name}},

Just a heads-up that your account is on the {{plan}} plan.

Sent from a DB Free workflow`,
  },
  {
    label: "Plain summary",
    description: "The row count and a text table of the rows.",
    subject: "{{rows.count}} rows in today's run",
    body: `Today's run produced {{rows.count}} rows:

{{rows.table}}

Sent from a DB Free workflow`,
  },
  {
    label: "HTML greeting",
    description: "A one-person HTML message.",
    subject: "Hi {{name}}",
    body: `<p>Hi {{name}},</p>
<p>Your account is on the <strong>{{plan}}</strong> plan.</p>
<p style="color:#666;font-size:12px">Sent from a DB Free workflow</p>`,
  },
  {
    label: "HTML report",
    description: "The row count and an HTML table of the rows.",
    subject: "{{rows.count}} rows in today's run",
    body: `<p>Today's run produced <strong>{{rows.count}}</strong> rows.</p>
{{rows.html}}
<p style="color:#666;font-size:12px">Sent from a DB Free workflow</p>`,
  },
  {
    label: "HTML alert",
    description: "A red-bordered alert box with the matching rows.",
    subject: "Alert: {{rows.count}} matching rows",
    body: `<div style="border-left:4px solid #d04040;padding:12px 14px;background:#fff5f5;font-family:sans-serif">
  <p style="margin:0 0 8px;font-weight:600">{{rows.count}} matching rows</p>
  {{rows.html}}
</div>`,
  },
];
