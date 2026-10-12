// SOT: relation-inspector, foreign-key-editor, composite-key-editor, relation-line-style
import type { ConnectionColor, DiagramBody, DiagramRelation, DiagramSide, DiagramTable, ReferentialAction } from "@/lib/bindings";
import { AppSelect, Field, Segmented, type Option } from "@/components/global/Field";
import { Button } from "@/components/ui/button";
import { IconButton } from "@/components/global/Button";
import { Icon } from "@/lib/icons";
import { ColorSwatches } from "./ColorSwatches";
import { linksOf, removeRelations, setRelationLinks, swapRelation, updateRelation } from "./designerModel";
import { InspectorPanel, LabeledRow, Section } from "./InspectorParts";
import { LINE_WIDTHS, LINE_WIDTH_ORDER } from "./palette";
import type { Edit } from "./TableInspector";

type ActionChoice = "default" | ReferentialAction;
type SideChoice = "auto" | DiagramSide;

const ACTION_OPTIONS: readonly Option<ActionChoice>[] = [
  { value: "default", label: "Default" },
  { value: "no_action", label: "NO ACTION" },
  { value: "restrict", label: "RESTRICT" },
  { value: "cascade", label: "CASCADE" },
  { value: "set_null", label: "SET NULL" },
  { value: "set_default", label: "SET DEFAULT" },
];

const SIDE_OPTIONS: readonly Option<SideChoice>[] = [
  { value: "auto", label: "Automatic" },
  { value: "top", label: "Top" },
  { value: "right", label: "Right" },
  { value: "bottom", label: "Bottom" },
  { value: "left", label: "Left" },
];

/// The select cannot hold an empty value, so "no column chosen yet" is this.
const NO_COLUMN = "\u0000none";

function columnOptions(table: DiagramTable | undefined): readonly Option<string>[] {
  return [{ value: NO_COLUMN, label: "Choose a column" }, ...(table?.columns ?? []).filter((c) => c.name !== "").map((c) => ({ value: c.name, label: c.name }))];
}

interface RelationInspectorProps {
  relation: DiagramRelation;
  body: DiagramBody;
  edit: Edit;
  onClose: () => void;
}

// WHAT:  The side panel for one relation: which columns it joins (one pair, or
//        several for a composite key), what happens on delete and update, the
//        constraint name, which sides of the cards it attaches to, and how the
//        line is drawn.
// WHY:   Drawing a line says "these tables are related"; the columns and actions
//        are what make it a foreign key, and they have to be chosen somewhere.
export function RelationInspector({ relation, body, edit, onClose }: RelationInspectorProps) {
  const from = body.tables.find((t) => t.id === relation.fromTable);
  const to = body.tables.find((t) => t.id === relation.toTable);
  const links = linksOf(relation);
  const setLinks = (next: readonly { from: string; to: string }[]) => {
    edit((b) => setRelationLinks(b, relation.id, next));
  };
  const patch = (changes: Partial<DiagramRelation>) => {
    edit((b) => updateRelation(b, relation.id, changes));
  };

  return (
    <InspectorPanel
      title="Relation"
      onClose={onClose}
      footer={
        <>
          <Button
            size="sm"
            variant="secondary"
            onClick={() => {
              edit((b) => swapRelation(b, relation.id));
            }}
          >
            <Icon name="exchange" size={13} />
            Swap direction
          </Button>
          <Button
            size="sm"
            variant="danger-soft"
            className="ml-auto"
            onClick={() => {
              edit((b) => removeRelations(b, new Set([relation.id])));
              onClose();
            }}
          >
            <Icon name="trash" size={13} />
            Delete
          </Button>
        </>
      }
    >
      <p className="flex items-center gap-1.5 font-mono text-xs text-foreground">
        <span className="truncate">{from?.name ?? "?"}</span>
        <Icon name="chevron-right" size={12} className="shrink-0 text-muted" />
        <span className="truncate">{to?.name ?? "?"}</span>
      </p>

      <Section
        title="Columns"
        action={
          <Button
            size="xs"
            variant="ghost"
            onClick={() => {
              setLinks([...links, { from: "", to: "" }]);
            }}
          >
            <Icon name="plus" size={12} />
            Add pair
          </Button>
        }
      >
        <div className="flex flex-col gap-2">
          {links.map((link, index) => (
            <div key={index} className="grid grid-cols-[1fr_1fr_auto] items-end gap-1.5">
              <AppSelect
                ariaLabel={`Pair ${index + 1} column in ${from?.name ?? "the first table"}`}
                size="sm"
                value={link.from === "" ? NO_COLUMN : link.from}
                options={columnOptions(from)}
                onChange={(value) => {
                  setLinks(links.map((l, i) => (i === index ? { ...l, from: value === NO_COLUMN ? "" : value } : l)));
                }}
              />
              <AppSelect
                ariaLabel={`Pair ${index + 1} column in ${to?.name ?? "the second table"}`}
                size="sm"
                value={link.to === "" ? NO_COLUMN : link.to}
                options={columnOptions(to)}
                onChange={(value) => {
                  setLinks(links.map((l, i) => (i === index ? { ...l, to: value === NO_COLUMN ? "" : value } : l)));
                }}
              />
              <IconButton
                icon="x"
                label="Remove pair"
                size={12}
                disabled={links.length === 1}
                onClick={() => {
                  setLinks(links.filter((_, i) => i !== index));
                }}
              />
            </div>
          ))}
          <p className="text-[11px] text-muted">The left column references the right one: {from?.name ?? "?"} → {to?.name ?? "?"}.</p>
        </div>
      </Section>

      <Section title="Actions">
        <div className="grid grid-cols-2 gap-2">
          <AppSelect
            label="On delete"
            size="sm"
            value={relation.onDelete ?? "default"}
            options={ACTION_OPTIONS}
            onChange={(choice) => {
              patch({ onDelete: choice === "default" ? null : choice });
            }}
          />
          <AppSelect
            label="On update"
            size="sm"
            value={relation.onUpdate ?? "default"}
            options={ACTION_OPTIONS}
            onChange={(choice) => {
              patch({ onUpdate: choice === "default" ? null : choice });
            }}
          />
        </div>
        <Field
          label="Constraint name"
          compact
          mono
          optional
          placeholder="generated"
          value={relation.name ?? ""}
          onChange={(name) => {
            patch({ name: name === "" ? null : name });
          }}
        />
      </Section>

      <Section title="Line">
        <LabeledRow label="Colour">
          <ColorSwatches
            value={relation.color}
            resetLabel="Accent"
            onChange={(color: ConnectionColor | null) => {
              patch({ color });
            }}
          />
        </LabeledRow>
        <LabeledRow label="Width">
          <Segmented
            label="Line width"
            value={relation.width ?? "normal"}
            options={LINE_WIDTH_ORDER.map((width) => ({ value: width, label: LINE_WIDTHS[width].label }))}
            onChange={(width) => {
              patch({ width });
            }}
          />
        </LabeledRow>
        <div className="grid grid-cols-2 gap-2">
          <AppSelect
            label="Leaves from"
            size="sm"
            value={relation.fromSide ?? "auto"}
            options={SIDE_OPTIONS}
            onChange={(side) => {
              patch({ fromSide: side === "auto" ? null : side });
            }}
          />
          <AppSelect
            label="Arrives at"
            size="sm"
            value={relation.toSide ?? "auto"}
            options={SIDE_OPTIONS}
            onChange={(side) => {
              patch({ toSide: side === "auto" ? null : side });
            }}
          />
        </div>
        <div className="flex items-center justify-between gap-2">
          <p className="text-[11px] text-muted">
            {relation.waypoints.length === 0
              ? "Double-click the line to add a bend."
              : `${relation.waypoints.length} bend point${relation.waypoints.length === 1 ? "" : "s"}.`}
          </p>
          {relation.waypoints.length > 0 ? (
            <Button
              size="xs"
              variant="ghost"
              onClick={() => {
                patch({ waypoints: [] });
              }}
            >
              Straighten
            </Button>
          ) : null}
        </div>
      </Section>
    </InspectorPanel>
  );
}
