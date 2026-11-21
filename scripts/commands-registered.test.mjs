// node --test scripts/
// Каждая команда, которую вызывает интерфейс (invoke("имя")), должна быть
// зарегистрирована в программе. Иначе вызов молча падает: так в 0.9.0
// замер скорости прокси всегда показывал «не замерить».
import assert from "node:assert/strict";
import { test } from "node:test";
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";

function files(dir, out = []) {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) files(p, out);
    else if (/\.(ts|tsx)$/.test(name)) out.push(p);
  }
  return out;
}

/** Имена из всех generate_handler![...] в src-tauri/src. */
function registered() {
  const names = new Set();
  for (const p of readdirSync("src-tauri/src", { recursive: true })) {
    const full = join("src-tauri/src", String(p));
    if (!full.endsWith(".rs")) continue;
    const src = readFileSync(full, "utf8");
    for (const m of src.matchAll(/generate_handler!\[([\s\S]*?)\]/g)) {
      const body = m[1].replace(/\/\/.*$/gm, "");
      for (const item of body.split(",")) {
        const name = item.trim().split("::").pop();
        if (name) names.add(name);
      }
    }
  }
  return names;
}

test("все команды интерфейса зарегистрированы в программе", () => {
  const reg = registered();
  assert.ok(reg.size > 50, `нашлось мало команд: ${reg.size}`);
  const missing = new Set();
  for (const f of files("src")) {
    const src = readFileSync(f, "utf8");
    for (const m of src.matchAll(/invoke(?:<[^>]*>)?\(\s*"([a-z_0-9]+)"/g)) {
      if (!reg.has(m[1])) missing.add(`${m[1]} (${f})`);
    }
  }
  assert.deepEqual([...missing], []);
});
