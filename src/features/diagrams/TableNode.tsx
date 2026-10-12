// SOT: table-node, erd-node, diagram-node-rendering, table-card
import { Handle, Position, type Node, type NodeProps } from "@xyflow/react";
import type { ConnectionColor, DiagramSide } from "@/lib/bindings";
import { Icon, type IconName } from "@/lib/icons";
import { cn } from "@/lib/cn";
import { BORDER, EMPTY_HEIGHT, HEADER_HEIGHT, MAX_ERD_ROWS, MORE_HEIGHT, ROW_HEIGHT, TABLE_WIDTH } from "./metrics";
import { colorVar, inkVar } from "./palette";

export interface CardColumn {
  name: string;
  type: string;
  pk: boolean;
  /// What the column references ("→ users.id"), or null when it is not a foreign key.
  fk: string | null;
  unique: boolean;
  icon: IconName;
}

// React Flow node data must be assignable to its record constraint; a type alias
// (implicit index signature) satisfies that where an interface cannot.
// eslint-disable-next-line @typescript-eslint/consistent-type-definitions
export type TableNodeData = {
  title: string;
  schema: string | null;
  /// Header colour; null = the accent.
  color: ConnectionColor | null;
  columns: CardColumn[];
  /// Faded out (designer "focus selected" mode).
  dimmed: boolean;
  /// ER cards cap their rows and cannot be wired by hand; designer cards show every
  /// column and let a relation be started from a column's edge.
  mode: "erd" | "designer";
};

export type TableFlowNode = Node<TableNodeData, "table">;

const SIDES = [
  { id: "top", position: Position.Top },
  { id: "right", position: Position.Right },
  { id: "bottom", position: Position.Bottom },
  { id: "left", position: Position.Left },
] as const;

// WHAT:  The handle id of a column's connector on its left or right edge.
// WHY:   Dragging from a column to another column starts a relation with those
//        columns already chosen. Ids carry the row index; they only live for the
//        length of the drag, so a later reorder cannot make one stale.
export function columnHandleId(index: number, side: "l" | "r"): string {
  return `c${index}${side}`;
}

// WHAT:  The row a column handle id names, or null for one of the four side handles.
export function columnOfHandle(handleId: string | null | undefined): number | null {
  const match = /^c(\d+)[lr]$/.exec(handleId ?? "");
  const index = match?.[1];
  return index === undefined ? null : Number(index);
}

// WHAT:  The side of the card a handle sits on: one of the four side handles by its
//        id, or a column's left / right connector. Null for anything else.
export function sideOfHandle(handleId: string | null | undefined): DiagramSide | null {
  switch (handleId) {
    case "top":
    case "right":
    case "bottom":
    case "left":
      return handleId;
    default:
      break;
  }
  const match = /^c\d+([lr])$/.exec(handleId ?? "");
  if (match?.[1] === "l") return "left";
  if (match?.[1] === "r") return "right";
  return null;
}

// WHAT:  A table card: a coloured header, one 22px row per column, and four
//        connection points (one per side) that relations attach to.
// WHY:   Relations attach to a side of the card, not to a column row, so renaming
//        or removing a column never detaches a line; which columns a relation joins
//        is data on the relation. Row and header heights are fixed because the
//        layout and the image export size cards from the model (see metrics.ts).
export function TableNode({ data, selected }: NodeProps<TableFlowNode>) {
  const designer = data.mode === "designer";
  const shown = designer ? data.columns : data.columns.slice(0, MAX_ERD_ROWS);
  const hidden = data.columns.length - shown.length;
  const fill = data.color === null ? "var(--accent)" : colorVar(data.color);
  const ink = data.color === null ? "var(--accent-foreground)" : inkVar(data.color);
  return (
    <div
      className={cn(
        "group/card relative rounded-md border bg-surface text-[11px] shadow-md transition-opacity",
        selected ? "border-accent ring-1 ring-accent" : "border-border",
        data.dimmed ? "opacity-25" : "",
      )}
      style={{ width: TABLE_WIDTH }}
    >
      {SIDES.map((side) => (
        <Handle
          key={side.id}
          id={side.id}
          type="source"
          position={side.position}
          isConnectable={designer}
          className="!size-2 !border-0"
          style={{ background: fill }}
        />
      ))}
      {designer
        ? shown.flatMap((_, i) =>
            (["l", "r"] as const).map((edge) => (
              <Handle
                key={columnHandleId(i, edge)}
                id={columnHandleId(i, edge)}
                type="source"
                position={edge === "l" ? Position.Left : Position.Right}
                className="!size-2 !border-0 opacity-0 transition-opacity group-hover/card:opacity-100"
                style={{ top: HEADER_HEIGHT + BORDER / 2 + i * ROW_HEIGHT + ROW_HEIGHT / 2, background: fill }}
              />
            )),
          )
        : null}
      <div className="overflow-hidden rounded-[5px]">
        <div className="flex items-center gap-1.5 px-2 font-medium" style={{ height: HEADER_HEIGHT, background: fill, color: ink }}>
          <Icon name="table" size={12} />
          <span className="truncate">{data.title}</span>
          {data.schema !== null ? <span className="ml-auto truncate pl-2 text-[10px] opacity-75">{data.schema}</span> : null}
        </div>
        {data.columns.length === 0 ? (
          <p className="flex items-center border-t border-separator px-2 text-[10px] text-muted italic" style={{ height: EMPTY_HEIGHT }}>
            no columns
          </p>
        ) : (
          <ul>
            {shown.map((c, i) => (
              <li key={`${c.name}:${i}`} className="flex items-center gap-1.5 border-t border-separator px-2 text-muted" style={{ height: ROW_HEIGHT }}>
                <Icon name={c.fk !== null && !c.pk ? "link" : c.icon} size={11} className={c.pk ? "text-warning" : c.fk !== null ? "text-accent" : ""} />
                <span className="truncate text-foreground">{c.name === "" ? "unnamed" : c.name}</span>
                {c.unique && !c.pk ? <span className="text-[9px] font-semibold text-muted">UQ</span> : null}
                <span className="ml-auto truncate pl-2 font-mono text-[10px]">{c.type}</span>
              </li>
            ))}
            {hidden > 0 ? (
              <li className="flex items-center border-t border-separator px-2 text-[10px] text-muted" style={{ height: MORE_HEIGHT }}>
                +{hidden} more
              </li>
            ) : null}
          </ul>
        )}
      </div>
    </div>
  );
}
