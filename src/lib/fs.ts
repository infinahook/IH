import type { FileNode } from "./api";

export interface TreeNode {
  node: FileNode;
  children: TreeNode[];
}

/** Assemble la liste plate de `file_tree` en arbre hiérarchique trié. */
export function buildFileTree(nodes: FileNode[]): TreeNode[] {
  const byPath = new Map<string, TreeNode>();
  for (const node of nodes) {
    byPath.set(node.path, { node, children: [] });
  }
  const roots: TreeNode[] = [];
  for (const node of nodes) {
    const entry = byPath.get(node.path);
    if (!entry) {
      continue;
    }
    const slash = node.path.lastIndexOf("/");
    const parentPath = slash >= 0 ? node.path.slice(0, slash) : "";
    const parent = parentPath ? byPath.get(parentPath) : undefined;
    if (parent) {
      parent.children.push(entry);
    } else {
      roots.push(entry);
    }
  }
  const sort = (entries: TreeNode[]): void => {
    entries.sort((left, right) => {
      if (left.node.is_dir !== right.node.is_dir) {
        return left.node.is_dir ? -1 : 1;
      }
      return left.node.name.localeCompare(right.node.name);
    });
    for (const entry of entries) {
      sort(entry.children);
    }
  };
  sort(roots);
  return roots;
}

/** Décalage du premier caractère d'une ligne (1-based) dans un texte. */
export function lineOffset(content: string, line: number): number {
  if (line <= 1) {
    return 0;
  }
  let offset = 0;
  let current = 1;
  while (current < line) {
    const next = content.indexOf("\n", offset);
    if (next === -1) {
      return content.length;
    }
    offset = next + 1;
    current += 1;
  }
  return offset;
}
