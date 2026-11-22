import test from 'node:test';
import fs from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { lua, lauxlib, lualib, to_luastring, to_jsstring } from 'fengari';
const root = fileURLToPath(new URL('../', import.meta.url));

async function run(file) {
  const state = lauxlib.luaL_newstate();
  lualib.luaL_openlibs(state);
  try {
    const source = await fs.readFile(new URL(file, import.meta.url), 'utf8');
    lua.lua_pushstring(state, to_luastring(root.replaceAll('\\', '/').replace(/\/$/, '')));
    lua.lua_setglobal(state, to_luastring('ASTRAL_SOURCE_ROOT'));
    const status = lauxlib.luaL_loadbuffer(state, to_luastring(source), null, to_luastring(file));
    if (status !== lua.LUA_OK) throw Error(to_jsstring(lua.lua_tostring(state, -1)));
    if (lua.lua_pcall(state, 0, 0, 0) !== lua.LUA_OK)
      throw Error(to_jsstring(lua.lua_tostring(state, -1)));
  } finally {
    lua.lua_close(state);
  }
}

test('in-game desktop/handheld transitions preserve profiles, controller bars and combat guards', () =>
  run('./lua/forever-profile-bars.lua'));
test('Forever initialization preserves live table identity, independent bars and later manual choices', () =>
  run('./lua/forever-style.lua'));
