// SOT: safe-expression-evaluator, text-tile-arithmetic, expression-parser
// WHAT:  A small arithmetic evaluator for `{{a / b * 100}}` in text tiles: numbers,
//        column names, + - * / %, parentheses, unary minus, and round / floor /
//        ceil / abs / min / max.
// WHY:   Text tiles show a value from the query's first row; people want a
//        ratio or a percentage of two columns without rewriting the SQL. The
//        text is the user's own, but it is still parsed here rather than handed
//        to `eval` / `Function`: nothing in it can reach anything but numbers.
// HOW:   A tokenizer and a recursive-descent parser build a tree, which is then
//        evaluated against a column lookup. Length and nesting are capped, so a
//        pathological expression fails instead of exhausting the stack. A bad
//        expression (syntax, a column that does not exist) returns null, so the
//        tile can show the text as typed; a bad *result* (divide by zero, a
//        non-numeric cell) is a number that is not finite.
// WHERE: ./charts.tsx (TextWidget substitution)

export interface ExpressionResult {
  value: number;
  /// Set when the whole expression is `round(x, n)`: the digits to print.
  decimals: number | null;
}

/// A column's number: NaN when the cell is not numeric, undefined when there is no such column.
export type ColumnLookup = (name: string) => number | undefined;

const MAX_LENGTH = 400;
const MAX_DEPTH = 40;

type Token =
  | { kind: "num"; value: number }
  | { kind: "name"; value: string }
  | { kind: "op"; value: "+" | "-" | "*" | "/" | "%" | "(" | ")" | "," };

function tokenize(source: string): Token[] | null {
  const tokens: Token[] = [];
  let i = 0;
  while (i < source.length) {
    const ch = source.charAt(i);
    if (ch === " " || ch === "\t" || ch === "\n") {
      i += 1;
      continue;
    }
    if (/[0-9.]/.test(ch)) {
      const hit = /^(?:\d+\.?\d*|\.\d+)/.exec(source.slice(i));
      if (hit === null) return null;
      tokens.push({ kind: "num", value: Number(hit[0]) });
      i += hit[0].length;
      continue;
    }
    if (/[A-Za-z_]/.test(ch)) {
      const hit = /^[A-Za-z_][A-Za-z0-9_]*/.exec(source.slice(i));
      if (hit === null) return null;
      tokens.push({ kind: "name", value: hit[0] });
      i += hit[0].length;
      continue;
    }
    if (ch === "+" || ch === "-" || ch === "*" || ch === "/" || ch === "%" || ch === "(" || ch === ")" || ch === ",") {
      tokens.push({ kind: "op", value: ch });
      i += 1;
      continue;
    }
    return null;
  }
  return tokens;
}

type Node =
  | { kind: "num"; value: number }
  | { kind: "col"; name: string }
  | { kind: "neg"; operand: Node }
  | { kind: "bin"; op: "+" | "-" | "*" | "/" | "%"; left: Node; right: Node }
  | { kind: "call"; name: string; args: Node[] };

const FUNCTIONS: Readonly<Record<string, { min: number; max: number }>> = {
  round: { min: 1, max: 2 },
  floor: { min: 1, max: 1 },
  ceil: { min: 1, max: 1 },
  abs: { min: 1, max: 1 },
  min: { min: 1, max: 8 },
  max: { min: 1, max: 8 },
};

class Parser {
  private at = 0;
  private readonly tokens: readonly Token[];

  constructor(tokens: readonly Token[]) {
    this.tokens = tokens;
  }

  parse(): Node | null {
    const node = this.expression(0);
    return node !== null && this.at === this.tokens.length ? node : null;
  }

  private peek(): Token | undefined {
    return this.tokens[this.at];
  }

  /// The operator at the cursor, or null when the next token is not one.
  private peekOp(): Extract<Token, { kind: "op" }>["value"] | null {
    const token = this.peek();
    return token?.kind === "op" ? token.value : null;
  }

  private isOp(value: string): boolean {
    return this.peekOp() === value;
  }

  private expression(depth: number): Node | null {
    if (depth > MAX_DEPTH) return null;
    let left = this.term(depth);
    for (let op = this.peekOp(); left !== null && (op === "+" || op === "-"); op = this.peekOp()) {
      this.at += 1;
      const right = this.term(depth);
      if (right === null) return null;
      left = { kind: "bin", op, left, right };
    }
    return left;
  }

  private term(depth: number): Node | null {
    let left = this.unary(depth);
    for (let op = this.peekOp(); left !== null && (op === "*" || op === "/" || op === "%"); op = this.peekOp()) {
      this.at += 1;
      const right = this.unary(depth);
      if (right === null) return null;
      left = { kind: "bin", op, left, right };
    }
    return left;
  }

  private unary(depth: number): Node | null {
    if (depth > MAX_DEPTH) return null;
    if (this.isOp("-")) {
      this.at += 1;
      const operand = this.unary(depth + 1);
      return operand === null ? null : { kind: "neg", operand };
    }
    if (this.isOp("+")) {
      this.at += 1;
      return this.unary(depth + 1);
    }
    return this.primary(depth);
  }

  private primary(depth: number): Node | null {
    const token = this.peek();
    if (token === undefined) return null;
    this.at += 1;
    if (token.kind === "num") return { kind: "num", value: token.value };
    if (token.kind === "name") {
      if (!this.isOp("(")) return { kind: "col", name: token.value };
      const spec = FUNCTIONS[token.value.toLowerCase()];
      if (spec === undefined) return null;
      this.at += 1;
      const args: Node[] = [];
      if (!this.isOp(")")) {
        for (;;) {
          const arg = this.expression(depth + 1);
          if (arg === null) return null;
          args.push(arg);
          if (this.isOp(",")) {
            this.at += 1;
            continue;
          }
          break;
        }
      }
      if (!this.isOp(")")) return null;
      this.at += 1;
      if (args.length < spec.min || args.length > spec.max) return null;
      return { kind: "call", name: token.value.toLowerCase(), args };
    }
    if (token.value === "(") {
      const inner = this.expression(depth + 1);
      if (inner === null || !this.isOp(")")) return null;
      this.at += 1;
      return inner;
    }
    return null;
  }
}

function roundTo(value: number, digits: number): number {
  const factor = 10 ** digits;
  return Math.round(value * factor) / factor;
}

/// null when a column the expression names does not exist.
function evaluate(node: Node, lookup: ColumnLookup): number | null {
  switch (node.kind) {
    case "num":
      return node.value;
    case "col":
      return lookup(node.name) ?? null;
    case "neg": {
      const v = evaluate(node.operand, lookup);
      return v === null ? null : -v;
    }
    case "bin": {
      const l = evaluate(node.left, lookup);
      const r = evaluate(node.right, lookup);
      if (l === null || r === null) return null;
      switch (node.op) {
        case "+":
          return l + r;
        case "-":
          return l - r;
        case "*":
          return l * r;
        case "/":
          return l / r;
        case "%":
          return l % r;
      }
      return null;
    }
    case "call": {
      const values: number[] = [];
      for (const arg of node.args) {
        const v = evaluate(arg, lookup);
        if (v === null) return null;
        values.push(v);
      }
      const first = values[0] ?? NaN;
      switch (node.name) {
        case "round":
          return roundTo(first, Math.max(0, Math.min(10, Math.trunc(values[1] ?? 0))));
        case "floor":
          return Math.floor(first);
        case "ceil":
          return Math.ceil(first);
        case "abs":
          return Math.abs(first);
        case "min":
          return Math.min(...values);
        case "max":
          return Math.max(...values);
      }
      return null;
    }
  }
}

/// null when the text is not a valid expression or names a missing column.
export function evaluateExpression(source: string, lookup: ColumnLookup): ExpressionResult | null {
  const text = source.trim();
  if (text.length === 0 || text.length > MAX_LENGTH) return null;
  const tokens = tokenize(text);
  if (tokens === null) return null;
  const tree = new Parser(tokens).parse();
  if (tree === null) return null;
  const value = evaluate(tree, lookup);
  if (value === null) return null;
  const decimals = tree.kind === "call" && tree.name === "round" ? Math.max(0, Math.min(10, Math.trunc(evaluate(tree.args[1] ?? { kind: "num", value: 0 }, lookup) ?? 0))) : null;
  return { value, decimals };
}

/// How a result reads in a tile: whole numbers as they are, `round(x, n)` with
/// n digits, anything else to two places. A result that is not finite is a dash.
export function formatExpressionResult(result: ExpressionResult): string {
  if (!Number.isFinite(result.value)) return "—";
  const text = result.decimals !== null ? result.value.toFixed(result.decimals) : Number.isInteger(result.value) ? String(result.value) : result.value.toFixed(2);
  return text === "-0" ? "0" : text;
}
