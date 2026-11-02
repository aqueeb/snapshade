// Snapshot undo stack (ARCHITECTURE Pattern 5). Stores whole-document snapshots;
// the document is tiny (tens of shapes) so this is cheaper and simpler than a
// command pattern. There is deliberately no way to re-apply a popped snapshot:
// redo is deferred to v2 (EDIT-02, ANNOT-11).
export class UndoStack<T> {
  private readonly items: T[] = [];

  push(snapshot: T): void {
    this.items.push(snapshot);
  }

  pop(): T | undefined {
    return this.items.pop();
  }

  get length(): number {
    return this.items.length;
  }
}
