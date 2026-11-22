export const now = () => Math.floor(Date.now() / 1000);
export const random = () => b64(crypto.getRandomValues(new Uint8Array(32)));
export function b64(bytes) {
  return btoa(String.fromCharCode(...bytes))
    .replaceAll('+', '-')
    .replaceAll('/', '_')
    .replaceAll('=', '');
}
export function unb64(s) {
  return Uint8Array.from(
    atob(s.replaceAll('-', '+').replaceAll('_', '/') + '='.repeat((4 - (s.length % 4)) % 4)),
    (c) => c.charCodeAt(0),
  );
}

export const json = (value, status = 200) =>
  Response.json(value, {
    status,
    headers: {
      'Cache-Control': 'no-store',
      'X-Content-Type-Options': 'nosniff',
      'Referrer-Policy': 'no-referrer',
    },
  });
export const query = (env, sql, ...args) => env.DB.prepare(sql).bind(...args);
export async function body(request, limit = 4096) {
  const size = Number(request.headers.get('content-length') || 0);
  if (size > limit) throw Error('Request too large.');
  const reader = request.body?.getReader();
  if (!reader) throw Error('Missing request.');
  let total = 0;
  const blocks = [];
  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    total += value.length;
    if (total > limit) {
      await reader.cancel();
      throw Error('Request too large.');
    }
    blocks.push(value);
  }
  const buffer = new Uint8Array(total);
  let at = 0;
  for (const b of blocks) {
    buffer.set(b, at);
    at += b.length;
  }
  return JSON.parse(new TextDecoder().decode(buffer));
}
