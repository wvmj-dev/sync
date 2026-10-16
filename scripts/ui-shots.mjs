// Capturas de tela da interface, claro e escuro, para revisar o visual (.agents/design.md).
// Usa o modo de demonstração (`?mock`, só em desenvolvimento) e o Chrome sem janela.
//
//   pnpm dev                                   # em outro terminal
//   node scripts/ui-shots.mjs /tmp/syncly-shots [filtro]
//
// O filtro escolhe capturas pelo nome (ex.: "escuro", "celular").
import { spawn } from "node:child_process";
import { mkdtempSync, writeFileSync, mkdirSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const out = process.argv[2];
const only = process.argv[3];
mkdirSync(out, { recursive: true });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const base = "http://localhost:1420/?mock";

const shots = [
  { name: "01-visao-geral-claro", theme: "light", route: "/" },
  { name: "02-visao-geral-escuro", theme: "dark", route: "/" },
  { name: "03-tarefas-claro", theme: "light", route: "/tarefas" },
  { name: "04-tarefas-escuro", theme: "dark", route: "/tarefas" },
  { name: "05-previa-claro", theme: "light", route: "/tarefas", click: ["Prévia"] },
  { name: "06-status-escuro", theme: "dark", route: "/tarefas", click: ["Status"] },
  { name: "07-versoes-claro", theme: "light", route: "/tarefas", click: ["Versões"] },
  { name: "08-nova-tarefa-escuro", theme: "dark", route: "/tarefas", click: ["Nova tarefa"] },
  { name: "09-contas-claro", theme: "light", route: "/contas" },
  { name: "10-arquivos-escuro", theme: "dark", route: "/arquivos" },
  { name: "11-compactar-claro", theme: "light", route: "/compactar" },
  { name: "12-celular-escuro", theme: "dark", route: "/", width: 390, height: 844 },
  { name: "13-celular-tarefas-claro", theme: "light", route: "/tarefas", width: 390, height: 844 },
  { name: "14-preferencias-escuro", theme: "dark", route: "/preferencias" },
  { name: "15-reconectar-contas-claro", theme: "light", route: "/contas", query: "&reconnect" },
  { name: "16-reconectar-visao-geral-escuro", theme: "dark", route: "/", query: "&reconnect" },
  { name: "17-reconectar-tarefas-claro", theme: "light", route: "/tarefas", query: "&reconnect" },
  { name: "18-adiada-tarefas-escuro", theme: "dark", route: "/tarefas", query: "&postponed" },
  { name: "19-adiada-visao-geral-claro", theme: "light", route: "/", query: "&postponed" },
  { name: "20-nova-pasta-claro", theme: "light", route: "/arquivos", click: ["Nova pasta"] },
  { name: "21-nova-pasta-celular-escuro", theme: "dark", route: "/arquivos", click: ["Nova pasta"], width: 390, height: 844 },
  { name: "22-criptografia-sem-chave-claro", theme: "light", route: "/preferencias", height: 1250 },
  { name: "23-criptografia-ativa-escuro", theme: "dark", route: "/preferencias", query: "&encrypted", height: 1250 },
  { name: "24-versoes-criptografadas-claro", theme: "light", route: "/tarefas", query: "&encrypted", click: ["Versões"] },
  { name: "25-compactar-criptografia-escuro", theme: "dark", route: "/compactar", query: "&encrypted", height: 1000 },
  { name: "29-troca-de-tela-claro", theme: "light", route: "/", click: ["Tarefas"] },
    // Por último: o menu recolhido fica salvo no navegador das capturas seguintes.
  { name: "26-tela-cheia-claro", theme: "light", route: "/", width: 1920, height: 1080 },
  { name: "27-tela-cheia-recolhido-escuro", theme: "dark", route: "/tarefas", click: ["Recolher menu"], width: 1920, height: 1080 },
  { name: "28-recolhido-claro", theme: "light", route: "/preferencias", click: ["Recolher menu"] },
];

const profile = mkdtempSync(join(tmpdir(), "syncly-shots-"));
const chrome = spawn(
  "google-chrome",
  [
    "--headless=new",
    "--disable-gpu",
    "--hide-scrollbars",
    "--no-first-run",
    "--remote-debugging-port=9333",
    `--user-data-dir=${profile}`,
    "about:blank",
  ],
  { stdio: "ignore" },
);

let targets;
for (let i = 0; i < 40; i++) {
  await sleep(250);
  try {
    targets = await (await fetch("http://127.0.0.1:9333/json")).json();
    if (targets.some((t) => t.type === "page")) break;
  } catch {}
}
const page = targets.find((t) => t.type === "page");
const ws = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((r) => ws.addEventListener("open", r));
let seq = 0;
const pending = new Map();
ws.addEventListener("message", (e) => {
  const msg = JSON.parse(e.data);
  if (msg.id && pending.has(msg.id)) {
    pending.get(msg.id)(msg);
    pending.delete(msg.id);
  }
});
const send = (method, params = {}) =>
  new Promise((resolve) => {
    const id = ++seq;
    pending.set(id, resolve);
    ws.send(JSON.stringify({ id, method, params }));
  });

await send("Page.enable");
for (const shot of shots) {
  if (only && !shot.name.includes(only)) continue;
  const width = shot.width ?? 1280;
  const height = shot.height ?? 860;
  await send("Emulation.setDeviceMetricsOverride", {
    width,
    height,
    deviceScaleFactor: 1,
    mobile: width < 600,
  });
  await send("Emulation.setEmulatedMedia", {
    features: [{ name: "prefers-color-scheme", value: shot.theme }],
  });
  await send("Page.navigate", {
    url: `${base}&theme=${shot.theme}${shot.query ?? ""}#${shot.route}`,
  });
  await sleep(1500);
  for (const text of shot.click ?? []) {
    await send("Runtime.evaluate", {
      expression: `[...document.querySelectorAll("button, a")].find(b => b.textContent.trim().includes(${JSON.stringify(text)}))?.click()`,
    });
    await sleep(900);
  }
  const metrics = await send("Page.getLayoutMetrics");
  const content = metrics.result.cssContentSize;
  const shotRes = await send("Page.captureScreenshot", {
    format: "png",
    captureBeyondViewport: true,
    clip: { x: 0, y: 0, width, height: Math.min(Math.max(content.height, height), 3200), scale: 1 },
  });
  writeFileSync(join(out, `${shot.name}.png`), Buffer.from(shotRes.result.data, "base64"));
  console.log("ok", shot.name);
}
ws.close();
chrome.kill();
