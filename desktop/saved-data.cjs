// Accept the data grammar emitted by WoW's SavedVariables writer, never Lua code.
function parse(source, replacements = new Map()) {
  if (typeof source !== 'string' || Buffer.byteLength(source) > 8 * 1024 ** 2)
    throw Error('Saved settings exceed the 8 MB file limit.');
  let at = 0,
    count = 0,
    look;
  const edits = [],
    globals = [],
    globalNames = new Set();
  const keywords = new Set(
    'and break do else elseif end for function if in local nil not or repeat return then true false until while'.split(
      ' ',
    ),
  );
  const fail = () => {
    throw Error('Only saved addon data can be shared; executable Lua is not supported.');
  };
  function long() {
    const match = source.slice(at).match(/^\[(=*)\[/);
    if (!match) return null;
    const start = at;
    at += match[0].length;
    const end = source.indexOf(']' + match[1] + ']', at);
    if (end < 0) fail();
    let value = source.slice(at, end).replace(/^\r?\n/, '');
    at = end + match[1].length + 2;
    return { type: 'string', value, start, end: at };
  }
  function next() {
    if (++count > 2000000) fail();
    for (;;) {
      while (/\s/.test(source[at] || '') && at < source.length) at++;
      if (!source.startsWith('--', at)) break;
      at += 2;
      if (!long()) {
        const end = source.indexOf('\n', at);
        at = end < 0 ? source.length : end + 1;
      }
    }
    const start = at,
      ch = source[at];
    if (!ch) return { type: 'end' };
    if (ch === '[') {
      const token = long();
      if (token) return token;
    }
    if (ch === '"' || ch === "'") {
      const quote = source[at++],
        begin = at;
      while (at < source.length && source[at] !== quote) {
        const c = source[at++];
        if (c === '\n' || c === '\r' || c === '\0') fail();
        if (c === '\\') {
          if (at === source.length) fail();
          if (source[at] === '\r' && source[at + 1] === '\n') at++;
          at++;
        }
      }
      if (source[at++] !== quote) fail();
      const body = source.slice(begin, at - 1),
        output = Buffer.allocUnsafe(Buffer.byteLength(body));
      let i = 0,
        length = 0;
      while (i < body.length) {
        const slash = body.indexOf('\\', i),
          end = slash < 0 ? body.length : slash;
        length += output.write(body.slice(i, end), length);
        if (slash < 0) break;
        i = slash + 1;
        const numeric = body.slice(i).match(/^\d{1,3}/);
        if (numeric) {
          const byte = Number(numeric[0]);
          if (byte > 255) fail();
          output[length++] = byte;
          i += numeric[0].length;
        } else {
          const escape = body[i++];
          const codes = {
            a: '\x07',
            b: '\b',
            f: '\f',
            n: '\n',
            r: '\r',
            t: '\t',
            v: '\x0b',
            '\\': '\\',
            '"': '"',
            "'": "'",
            '\n': '\n',
            '\r': '\n',
          };
          if (!Object.hasOwn(codes, escape)) fail();
          if (escape === '\r' && body[i] === '\n') i++;
          length += output.write(codes[escape], length);
        }
      }
      const bytes = output.subarray(0, length),
        value = bytes.toString('utf8');
      return {
        type: 'string',
        value: Buffer.from(value).equals(bytes) ? value : null,
        start,
        end: at,
      };
    }
    const identifier = source.slice(at).match(/^[A-Za-z_][A-Za-z0-9_]*/);
    if (identifier) {
      at += identifier[0].length;
      return { type: 'id', value: identifier[0] };
    }
    const number = source
      .slice(at)
      .match(/^-?(?:0[xX][a-fA-F0-9]+|(?:\d+\.?\d*|\.\d+)(?:[eE][+-]?\d+)?)/);
    if (number) {
      at += number[0].length;
      return { type: 'number' };
    }
    if ('{}[]=,;'.includes(ch)) {
      at++;
      return { type: ch };
    }
    fail();
  }
  function peek() {
    return look || (look = next());
  }
  function take(type) {
    const token = peek();
    if (type && token.type !== type) fail();
    look = null;
    return token;
  }
  function value(depth = 0) {
    if (depth > 64) fail();
    const token = take();
    if (token.type === 'string') {
      if (replacements.has(token.value))
        edits.push({ ...token, value: replacements.get(token.value) });
      return;
    }
    if (
      token.type === 'number' ||
      (token.type === 'id' && ['true', 'false', 'nil'].includes(token.value))
    )
      return;
    if (token.type !== '{') fail();
    while (peek().type !== '}') {
      if (peek().type === '[') {
        take('[');
        if (
          !['string', 'number'].includes(peek().type) &&
          !(peek().type === 'id' && ['true', 'false'].includes(peek().value))
        )
          fail();
        value(depth + 1);
        take(']');
        take('=');
        value(depth + 1);
      } else if (peek().type === 'id' && !['true', 'false', 'nil'].includes(peek().value)) {
        if (keywords.has(take('id').value)) fail();
        take('=');
        value(depth + 1);
      } else value(depth + 1);
      if ([',', ';'].includes(peek().type)) take();
      else if (peek().type !== '}') fail();
    }
    take('}');
  }
  while (peek().type !== 'end') {
    const name = take('id').value;
    if (keywords.has(name) || globalNames.has(name) || name.length > 128 || globals.length >= 512)
      fail();
    globalNames.add(name);
    globals.push(name);
    take('=');
    value();
    if (peek().type === ';') take(';');
  }
  const quote = (value) =>
    '"' +
    value.replace(/[\\"\x00-\x1f\x7f]/g, (c) =>
      c === '\\' || c === '"' ? '\\' + c : '\\' + c.charCodeAt(0).toString().padStart(3, '0'),
    ) +
    '"';
  let text = source;
  for (const edit of edits.reverse())
    text = text.slice(0, edit.start) + quote(edit.value) + text.slice(edit.end);
  return { text, globals };
}
module.exports = { parse };
