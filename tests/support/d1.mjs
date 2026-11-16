import { DatabaseSync } from 'node:sqlite';

export class DB {
  constructor(sql) {
    this.db = new DatabaseSync(':memory:');
    this.db.exec(sql);
  }
  prepare(sql) {
    const native = this.db.prepare(sql);
    let v = [];
    const q = {
      bind(...args) {
        v = args;
        return q;
      },
      async first() {
        return native.get(...v) || null;
      },
      async all() {
        return { results: native.all(...v) };
      },
      async run() {
        const r = native.run(...v);
        return { meta: { changes: Number(r.changes) } };
      },
    };
    return q;
  }
  async batch(items) {
    this.db.exec('BEGIN');
    try {
      const r = [];
      for (const q of items) r.push(await q.run());
      this.db.exec('COMMIT');
      return r;
    } catch (e) {
      this.db.exec('ROLLBACK');
      throw e;
    }
  }
}
