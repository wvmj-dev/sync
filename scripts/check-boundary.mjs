// Confere a fronteira Rust ↔ webview, como na revisão de .agents/papeis/:
// - SEG-30: cada comando Tauri aparece igual em commands.rs, generate_handler! (main.rs),
//   COMMANDS (build.rs), capabilities/default.json e nas chamadas de ui/src/api/index.ts;
// - SEG-32: nada de v-html/innerHTML, `invoke` só em ui/src/api/, sem style="" no HTML
//   (a CSP de produção bloqueia estilo inline).
//
//   node scripts/check-boundary.mjs
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";

const read = (path) => readFileSync(path, "utf8");
const names = (text, re) => new Set([...text.matchAll(re)].map((m) => m[1]));
const problems = [];

/** Trecho entre `start` e o primeiro `end` depois dele. */
function block(text, start, end, file) {
  const i = text.indexOf(start);
  const j = i < 0 ? -1 : text.indexOf(end, i);
  if (j < 0) throw new Error(`${file}: não encontrei "${start}"`);
  return text.slice(i, j);
}

const sources = {
  "src-tauri/src/commands.rs (#[tauri::command])": names(
    read("src-tauri/src/commands.rs"),
    /#\[tauri::command\]\s*pub (?:async )?fn (\w+)/g,
  ),
  "src-tauri/build.rs (COMMANDS)": names(
    block(read("src-tauri/build.rs"), "COMMANDS", "];", "build.rs"),
    /"(\w+)"/g,
  ),
  "src-tauri/capabilities/default.json (allow-*)": new Set(
    JSON.parse(read("src-tauri/capabilities/default.json"))
      .permissions.filter((p) => typeof p === "string" && p.startsWith("allow-"))
      .map((p) => p.slice("allow-".length).replaceAll("-", "_")),
  ),
  "ui/src/api/index.ts (call)": names(read("ui/src/api/index.ts"), /\bcall(?:<[^(]*>)?\(\s*"(\w+)"/g),
};
const handler = names(
  block(read("src-tauri/src/main.rs"), "generate_handler![", "]", "main.rs"),
  /commands::(\w+)/g,
);

for (const [where, found] of Object.entries(sources)) {
  const missing = [...handler].filter((c) => !found.has(c));
  const extra = [...found].filter((c) => !handler.has(c));
  if (missing.length) problems.push(`${where}: falta ${missing.join(", ")}`);
  if (extra.length) problems.push(`${where}: não está no generate_handler!: ${extra.join(", ")}`);
}

function* files(dir) {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) yield* files(path);
    else if (/\.(vue|ts|html)$/.test(entry.name)) yield path;
  }
}

for (const path of [...files("ui/src"), "ui/index.html"]) {
  const text = read(path);
  if (/\bv-html\b|\binnerHTML\b/.test(text)) problems.push(`${path}: v-html/innerHTML (SEG-32)`);
  if (/\binvoke\b/.test(text) && !path.startsWith(join("ui", "src", "api"))) {
    problems.push(`${path}: invoke fora de ui/src/api/ (SEG-32)`);
  }
  if (/\sstyle="/.test(text)) problems.push(`${path}: style="" bloqueado pela CSP — use classe ou :style`);
}

if (problems.length) {
  console.error("Fronteira Rust ↔ webview com problemas:\n" + problems.map((p) => `  - ${p}`).join("\n"));
  process.exit(1);
}
console.log(`Fronteira consistente: ${handler.size} comandos nos 5 lugares; UI sem HTML cru.`);
