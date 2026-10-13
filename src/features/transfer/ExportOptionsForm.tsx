// SOT: export-options-form, export-format-picker, delimiter-picker, json-shape-picker, null-text-field
import type { ExportFormat, ExportJsonShape, ExportOptions } from "@/lib/bindings";
import { EXPORT_FORMAT_FACTS, EXPORT_FORMATS } from "@/lib/export";
import { AppSelect, Field, Segmented, Toggle } from "@/components/global/Field";

// WHAT:  The controls of an export: the format and, for each format, only the
//        options that format has.
// WHY:   The row export dialog and the multi-table export tab offer the same
//        knobs; one form keeps their wording and behaviour identical.
// HOW:   A value-in, value-out component. `structure` adds the include-structure
//        and include-data switches the multi-table export has (a row export has
//        no CREATE TABLE to include).
// WHERE: src/features/transfer/ExportDialog.tsx, src/features/transfer/ExportPanel.tsx
const DELIMITERS: readonly { value: string; label: string }[] = [
  { value: ",", label: "Comma  ," },
  { value: ";", label: "Semicolon  ;" },
  { value: "|", label: "Pipe  |" },
  { value: "\t", label: "Tab" },
];

interface ExportOptionsFormProps {
  options: ExportOptions;
  onChange: (options: ExportOptions) => void;
  /// Show include-structure / include-data (multi-table export, SQL and JSON).
  structure?: boolean;
  /// Formats on offer (a result set with no table cannot be written as SQL INSERTs, say).
  formats?: readonly ExportFormat[];
  disabled?: boolean;
}

export function ExportOptionsForm({ options, onChange, structure = false, formats = EXPORT_FORMATS, disabled = false }: ExportOptionsFormProps) {
  const set = (part: Partial<ExportOptions>) => {
    onChange({ ...options, ...part });
  };
  const delimited = options.format === "csv" || options.format === "tsv";
  const json = options.format === "json";
  const sql = options.format === "sql";

  return (
    <div className="flex flex-col gap-2">
      <AppSelect label="Format" value={options.format} options={formats.map((f) => ({ value: f, label: EXPORT_FORMAT_FACTS[f].label }))} onChange={(format) => { set({ format }); }} disabled={disabled} />

      {options.format === "csv" ? (
        <AppSelect
          label="Delimiter"
          value={options.delimiter}
          options={DELIMITERS.some((d) => d.value === options.delimiter) ? DELIMITERS : [...DELIMITERS, { value: options.delimiter, label: options.delimiter }]}
          onChange={(delimiter) => { set({ delimiter }); }}
          disabled={disabled}
        />
      ) : null}

      {delimited || options.format === "markdown" ? (
        <>
          {delimited ? <Toggle checked={options.header} onChange={(header) => { set({ header }); }} label="Header row" /> : null}
          <Field label="Write NULL as" value={options.nullText} onChange={(nullText) => { set({ nullText }); }} placeholder="(empty)" mono disabled={disabled} />
        </>
      ) : null}

      {delimited ? (
        <Toggle checked={options.quoteFormulas} onChange={(quoteFormulas) => { set({ quoteFormulas }); }} label="Protect against spreadsheet formulas" description="Text starting with = + - @ is prefixed with ' so Excel does not run it." />
      ) : null}

      {json ? (
        <>
          <div className="flex flex-col gap-1.5">
            <span className="text-[11px] font-medium text-muted">JSON shape</span>
            <Segmented<ExportJsonShape>
              label="JSON shape"
              value={options.jsonShape}
              onChange={(jsonShape) => { set({ jsonShape }); }}
              options={[
                { value: "array", label: "Row array" },
                { value: "envelope", label: "Table envelope" },
              ]}
            />
          </div>
          <Toggle checked={options.pretty} onChange={(pretty) => { set({ pretty }); }} label="Pretty print" />
        </>
      ) : null}

      {structure && (sql || json) ? (
        <>
          <Toggle checked={options.includeSchema} onChange={(includeSchema) => { set({ includeSchema }); }} label={sql ? "Include CREATE TABLE" : "Include structure (__schema)"} />
          <Toggle checked={options.includeData} onChange={(includeData) => { set({ includeData }); }} label="Include data" />
        </>
      ) : null}

      <Toggle checked={options.bom} onChange={(bom) => { set({ bom }); }} label="UTF-8 byte-order mark" description="Excel needs it to read non-English text in a CSV." />
    </div>
  );
}
