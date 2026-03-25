export interface TreemapNodeInput {
  id: string;
  label: string;
  value?: number | null;
  colorValue?: number | null;
  meta?: string | null;
  note?: string | null;
  accent?: string | null;
  children?: TreemapNodeInput[];
}

interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}

interface NormalizedTreemapNode {
  path: string;
  depth: number;
  label: string;
  value: number;
  colorValue: number | null;
  meta: string | null;
  note: string | null;
  accent: string | null;
  source: TreemapNodeInput;
  children: NormalizedTreemapNode[];
}

export interface TreemapLayoutNode {
  path: string;
  depth: number;
  label: string;
  value: number;
  colorValue: number | null;
  meta: string | null;
  note: string | null;
  accent: string | null;
  source: TreemapNodeInput;
  hasChildren: boolean;
  x: number;
  y: number;
  width: number;
  height: number;
  share: number;
  area: number;
}

export interface TreemapLayoutResult {
  total: number;
  nodes: TreemapLayoutNode[];
  metricMin: number;
  metricMax: number;
}

interface BuildTreemapOptions {
  width: number;
  height: number;
  groupInset?: number;
  groupHeaderHeight?: number;
}

const DEFAULT_GROUP_INSET = 4;
const DEFAULT_GROUP_HEADER_HEIGHT = 28;

export function buildTreemapLayout(
  items: TreemapNodeInput[],
  {
    width,
    height,
    groupInset = DEFAULT_GROUP_INSET,
    groupHeaderHeight = DEFAULT_GROUP_HEADER_HEIGHT
  }: BuildTreemapOptions
): TreemapLayoutResult {
  const normalized = items
    .map((item, index) => normalizeNode(item, `${index}`, 0))
    .filter((item) => item.value > 0);

  const total = normalized.reduce((sum, item) => sum + item.value, 0);
  const metrics = collectMetrics(normalized);
  const nodes: TreemapLayoutNode[] = [];

  if (total > 0 && width > 0 && height > 0) {
    layoutLevel(normalized, { x: 0, y: 0, width, height }, total, nodes, {
      groupInset,
      groupHeaderHeight
    });
  }

  return {
    total,
    nodes,
    metricMin: metrics.min,
    metricMax: metrics.max
  };
}

function normalizeNode(node: TreemapNodeInput, path: string, depth: number): NormalizedTreemapNode {
  const children = (node.children ?? [])
    .map((child, index) => normalizeNode(child, `${path}.${index}`, depth + 1))
    .filter((child) => child.value > 0);

  const ownValue = Math.max(0, Number(node.value ?? 0));
  const childValue = children.reduce((sum, child) => sum + child.value, 0);

  return {
    path,
    depth,
    label: node.label,
    value: Math.max(ownValue, childValue),
    colorValue: Number.isFinite(node.colorValue) ? Number(node.colorValue) : null,
    meta: node.meta ?? null,
    note: node.note ?? null,
    accent: node.accent ?? null,
    source: node,
    children
  };
}

function collectMetrics(nodes: NormalizedTreemapNode[]): { min: number; max: number } {
  const values: number[] = [];

  walkNodes(nodes, (node) => {
    if (node.children.length > 0) {
      return;
    }

    values.push(node.colorValue ?? node.value);
  });

  if (values.length === 0) {
    return { min: 0, max: 1 };
  }

  return {
    min: Math.min(...values),
    max: Math.max(...values)
  };
}

function walkNodes(nodes: NormalizedTreemapNode[], visit: (node: NormalizedTreemapNode) => void) {
  for (const node of nodes) {
    visit(node);
    if (node.children.length > 0) {
      walkNodes(node.children, visit);
    }
  }
}

function layoutLevel(
  nodes: NormalizedTreemapNode[],
  rect: Rect,
  rootTotal: number,
  target: TreemapLayoutNode[],
  options: { groupInset: number; groupHeaderHeight: number }
) {
  if (nodes.length === 0 || rect.width <= 0 || rect.height <= 0) {
    return;
  }

  const positioned = partitionNodes(nodes, rect);

  for (const { node, rect: nodeRect } of positioned) {
    const area = nodeRect.width * nodeRect.height;
    const hasChildren = node.children.length > 0;

    target.push({
      path: node.path,
      depth: node.depth,
      label: node.label,
      value: node.value,
      colorValue: node.colorValue,
      meta: node.meta,
      note: node.note,
      accent: node.accent,
      source: node.source,
      hasChildren,
      x: nodeRect.x,
      y: nodeRect.y,
      width: nodeRect.width,
      height: nodeRect.height,
      share: rootTotal > 0 ? node.value / rootTotal : 0,
      area
    });

    if (!hasChildren) {
      continue;
    }

    const childRect = insetGroupRect(nodeRect, options.groupInset, options.groupHeaderHeight);
    if (childRect.width <= 12 || childRect.height <= 12) {
      continue;
    }

    layoutLevel(node.children, childRect, rootTotal, target, options);
  }
}

function insetGroupRect(rect: Rect, inset: number, headerHeight: number): Rect {
  const nextX = rect.x + inset;
  const nextY = rect.y + inset + Math.min(headerHeight, Math.max(18, rect.height * 0.16));
  const nextWidth = rect.width - inset * 2;
  const nextHeight = rect.height - inset * 2 - Math.min(headerHeight, Math.max(18, rect.height * 0.16));

  return {
    x: nextX,
    y: nextY,
    width: nextWidth,
    height: nextHeight
  };
}

function partitionNodes(nodes: NormalizedTreemapNode[], rect: Rect): Array<{ node: NormalizedTreemapNode; rect: Rect }> {
  if (nodes.length === 0) {
    return [];
  }

  if (nodes.length === 1) {
    return [{ node: nodes[0], rect }];
  }

  const sorted = [...nodes].sort((a, b) => b.value - a.value);
  const total = sorted.reduce((sum, node) => sum + node.value, 0);
  const splitIndex = chooseSplitIndex(sorted, total);
  const left = sorted.slice(0, splitIndex);
  const right = sorted.slice(splitIndex);
  const leftTotal = left.reduce((sum, node) => sum + node.value, 0);

  if (left.length === 0 || right.length === 0 || total <= 0) {
    return sorted.map((node, index) => ({
      node,
      rect: sliceRect(rect, index, sorted.length, isHorizontal(rect))
    }));
  }

  if (isHorizontal(rect)) {
    const leftWidth = rect.width * (leftTotal / total);
    return [
      ...partitionNodes(left, {
        x: rect.x,
        y: rect.y,
        width: leftWidth,
        height: rect.height
      }),
      ...partitionNodes(right, {
        x: rect.x + leftWidth,
        y: rect.y,
        width: rect.width - leftWidth,
        height: rect.height
      })
    ];
  }

  const topHeight = rect.height * (leftTotal / total);
  return [
    ...partitionNodes(left, {
      x: rect.x,
      y: rect.y,
      width: rect.width,
      height: topHeight
    }),
    ...partitionNodes(right, {
      x: rect.x,
      y: rect.y + topHeight,
      width: rect.width,
      height: rect.height - topHeight
    })
  ];
}

function chooseSplitIndex(nodes: NormalizedTreemapNode[], total: number): number {
  let running = 0;

  for (let index = 0; index < nodes.length; index += 1) {
    running += nodes[index].value;
    if (running >= total / 2) {
      return Math.max(1, index + 1);
    }
  }

  return Math.max(1, nodes.length - 1);
}

function isHorizontal(rect: Rect): boolean {
  return rect.width >= rect.height;
}

function sliceRect(rect: Rect, index: number, count: number, horizontal: boolean): Rect {
  if (horizontal) {
    const width = rect.width / count;
    return {
      x: rect.x + width * index,
      y: rect.y,
      width,
      height: rect.height
    };
  }

  const height = rect.height / count;
  return {
    x: rect.x,
    y: rect.y + height * index,
    width: rect.width,
    height
  };
}
