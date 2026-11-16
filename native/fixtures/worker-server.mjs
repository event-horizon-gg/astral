// Local HTTP contract fixture. Never uses a Cloudflare account, Battle.net,
// persistent database, real sessions, or a WoW installation.
import http from 'node:http';
import fs from 'node:fs/promises';
import { DB } from '../../tests/support/d1.mjs';
import worker, { hash } from '../../cloud/worker.mjs';

const db = new DB('');
const migrations = new URL('../../cloud/migrations/', import.meta.url);
for (const name of (await fs.readdir(migrations)).filter((name) => name.endsWith('.sql')).sort()) {
  db.db.exec(await fs.readFile(new URL(name, migrations), 'utf8'));
}
db.db.prepare('INSERT INTO users(id,name) VALUES(?,?)').run('shared-user', 'Fixture player');
db.db.prepare('INSERT INTO users(id,name) VALUES(?,?)').run('other-user', 'Other fixture player');
for (const [letter, user] of [
  ['a', 'shared-user'],
  ['b', 'shared-user'],
  ['c', 'other-user'],
]) {
  db.db
    .prepare(
      'INSERT INTO sessions(hash,user_id,expires,id,label,created,last_seen) VALUES(?,?,?,?,?,?,?)',
    )
    .run(
      await hash(letter.repeat(43)),
      user,
      Math.floor(Date.now() / 1000) + 3600,
      letter.repeat(43),
      `Fixture device ${letter}`,
      1,
      1,
    );
}
const env = {
  DB: db,
  PUBLIC_ORIGIN: 'https://astral.test',
  BNET_CLIENT_ID: 'fixture-client',
  BNET_CLIENT_SECRET: 'fixture-only-not-real-credentials',
};
const server = http.createServer(async (incoming, outgoing) => {
  try {
    const chunks = [];
    let length = 0;
    for await (const chunk of incoming) {
      length += chunk.length;
      if (length > 1600000) {
        outgoing.writeHead(413).end();
        return;
      }
      chunks.push(chunk);
    }
    const request = new Request(env.PUBLIC_ORIGIN + incoming.url, {
      method: incoming.method,
      headers: incoming.headers,
      body: chunks.length ? Buffer.concat(chunks) : undefined,
    });
    const response = await worker.fetch(request, env);
    outgoing.writeHead(response.status, Object.fromEntries(response.headers));
    outgoing.end(Buffer.from(await response.arrayBuffer()));
  } catch (error) {
    outgoing.writeHead(500, { 'content-type': 'application/json' });
    outgoing.end(JSON.stringify({ error: error.message }));
  }
});
await new Promise((resolve, reject) => {
  server.once('error', reject);
  server.listen(0, '127.0.0.1', resolve);
});
console.log(JSON.stringify({ origin: `http://127.0.0.1:${server.address().port}` }));
