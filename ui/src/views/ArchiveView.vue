<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { api, errorMessage, onArchiveProgress, pickArchive, pickFiles, pickFolder } from "../api";
import type { ArchiveFormat, ArchiveProgress, CompressResult, ExtractResult } from "../api/types";
import { archiveFormats as formats, formatBytes } from "../utils/format";

// Compactar
const inputs = ref<string[]>([]);
const format = ref<ArchiveFormat>("zip");
const name = ref("");
const outputDir = ref<string | null>(null);
const includeSensitive = ref(false);
const compressed = ref<CompressResult | null>(null);

// Descompactar
const archive = ref<string | null>(null);
const destDir = ref<string | null>(null);
const extracted = ref<ExtractResult | null>(null);

// Uma operação por vez (o serviço também garante isso).
const busy = ref<"compress" | "extract" | null>(null);
const progress = ref<ArchiveProgress | null>(null);
const error = ref<string | null>(null);

let stopListening: (() => void) | null = null;
onMounted(async () => {
  stopListening = await onArchiveProgress((p) => (progress.value = p));
});
onUnmounted(() => stopListening?.());

const percent = computed(() => {
  const p = progress.value;
  return p && p.bytesTotal > 0 ? Math.min(100, (p.bytesDone / p.bytesTotal) * 100) : null;
});
const ratio = computed(() => {
  const r = compressed.value;
  return r && r.bytesIn > 0 ? Math.round((1 - r.bytesOut / r.bytesIn) * 100) : 0;
});

function parentOf(path: string): string {
  const i = path.lastIndexOf("/");
  return i > 0 ? path.slice(0, i) : "/";
}

function addInputs(paths: string[]) {
  for (const p of paths) if (!inputs.value.includes(p)) inputs.value.push(p);
  // Sugere salvar ao lado da primeira entrada.
  if (!outputDir.value && inputs.value[0]) outputDir.value = parentOf(inputs.value[0]);
}

async function addFolder() {
  const folder = await pickFolder("Adicionar pasta");
  if (folder) addInputs([folder]);
}

async function addFiles() {
  addInputs(await pickFiles("Adicionar arquivos"));
}

async function chooseOutput() {
  outputDir.value = (await pickFolder("Salvar o arquivo compactado em…")) ?? outputDir.value;
}

async function chooseArchive() {
  const picked = await pickArchive("Escolher arquivo compactado");
  if (!picked) return;
  archive.value = picked;
  destDir.value ??= parentOf(picked);
}

async function chooseDest() {
  destDir.value = (await pickFolder("Descompactar em…")) ?? destDir.value;
}

async function run<T>(kind: "compress" | "extract", op: () => Promise<T>): Promise<T | null> {
  busy.value = kind;
  progress.value = null;
  error.value = null;
  try {
    return await op();
  } catch (e) {
    error.value = errorMessage(e);
    return null;
  } finally {
    busy.value = null;
  }
}

async function compress() {
  if (!outputDir.value) return;
  const outDir = outputDir.value;
  compressed.value = await run("compress", () =>
    api.compress({
      inputs: inputs.value,
      outputDir: outDir,
      name: name.value.trim() || null,
      format: format.value,
      includeSensitive: includeSensitive.value,
    }),
  );
}

async function extract() {
  if (!archive.value || !destDir.value) return;
  const [file, dir] = [archive.value, destDir.value];
  extracted.value = await run("extract", () => api.extract(file, dir));
}
</script>

<template>
  <header class="page-head">
    <h1>Compactar</h1>
  </header>

  <div v-if="error" class="banner banner-danger" role="alert">
    <span>{{ error }}</span>
    <button class="btn btn-ghost" type="button" @click="error = null">Fechar</button>
  </div>

  <section v-if="busy" class="card" aria-live="polite">
    <div class="run-head">
      <strong>{{ busy === "compress" ? "Compactando…" : "Descompactando…" }}</strong>
      <button class="btn btn-ghost" type="button" @click="api.cancelArchive()">Cancelar</button>
    </div>
    <div class="progress" role="progressbar">
      <div :style="{ width: `${percent ?? 100}%` }" :class="{ indeterminate: percent === null }"></div>
    </div>
    <p v-if="progress" class="muted small">
      {{ progress.filesDone }}<template v-if="progress.filesTotal"> de {{ progress.filesTotal }}</template>
      arquivo(s) · {{ formatBytes(progress.bytesDone) }}<template v-if="progress.bytesTotal">
        de {{ formatBytes(progress.bytesTotal) }}</template>
    </p>
    <p v-if="progress?.current" class="muted small run-current">
      <code>{{ progress.current }}</code>
    </p>
  </section>

  <section class="card form">
    <h2>Compactar pastas e arquivos</h2>

    <div class="field">
      <span>O que compactar</span>
      <ul v-if="inputs.length" class="list compact">
        <li v-for="(p, i) in inputs" :key="p" class="list-row">
          <code>{{ p }}</code>
          <button class="btn btn-ghost" type="button" @click="inputs.splice(i, 1)">Remover</button>
        </li>
      </ul>
      <div class="row">
        <button class="btn" type="button" @click="addFolder">+ Pasta</button>
        <button class="btn" type="button" @click="addFiles">+ Arquivos</button>
      </div>
    </div>

    <fieldset class="field">
      <legend>Formato</legend>
      <div class="row">
        <label v-for="f in formats" :key="f.value" class="choice">
          <input v-model="format" type="radio" :value="f.value" />
          <span><strong>{{ f.label }}</strong> <span class="muted small">— {{ f.hint }}</span></span>
        </label>
      </div>
    </fieldset>

    <div class="grid-2">
      <label class="field">
        <span>Nome do arquivo (opcional)</span>
        <input v-model="name" maxlength="200" placeholder="Padrão: nome da pasta" />
      </label>
      <div class="field">
        <span>Salvar em</span>
        <div class="row">
          <button class="btn" type="button" @click="chooseOutput">Escolher pasta…</button>
          <span class="muted small path-preview">{{ outputDir ?? "nenhuma pasta escolhida" }}</span>
        </div>
      </div>
    </div>

    <label class="choice">
      <input v-model="includeSensitive" type="checkbox" />
      <span>
        Incluir arquivos sensíveis (<code>.env</code>, chaves, <code>.ssh</code>…).
        <strong>Não recomendado</strong> se for compartilhar o arquivo.
      </span>
    </label>

    <div class="row end">
      <button
        class="btn btn-primary"
        type="button"
        :disabled="!inputs.length || !outputDir || !!busy"
        @click="compress"
      >
        Compactar
      </button>
    </div>

    <div v-if="compressed" class="banner banner-ok">
      <div>
        <strong>Criado:</strong> <code>{{ compressed.path }}</code>
        <p class="small">
          {{ compressed.files }} arquivo(s), {{ compressed.folders }} pasta(s) —
          {{ formatBytes(compressed.bytesIn) }} → {{ formatBytes(compressed.bytesOut) }}
          <template v-if="ratio > 0">({{ ratio }}% menor)</template>
          em {{ (compressed.durationMs / 1000).toFixed(1) }} s.
        </p>
        <details v-if="compressed.warnings.length">
          <summary>{{ compressed.warnings.length }} aviso(s)</summary>
          <ul class="small">
            <li v-for="w in compressed.warnings" :key="w">{{ w }}</li>
          </ul>
        </details>
      </div>
      <button class="btn btn-ghost" type="button" @click="compressed = null">Fechar</button>
    </div>
  </section>

  <section class="card form">
    <h2>Descompactar</h2>
    <p class="muted small">
      Aceita <code>.zip</code>, <code>.tar</code>, <code>.tar.gz</code> e <code>.tar.zst</code>.
      O conteúdo vai sempre para uma <strong>pasta nova</strong> — nada existente é sobrescrito.
    </p>

    <div class="grid-2">
      <div class="field">
        <span>Arquivo compactado</span>
        <div class="row">
          <button class="btn" type="button" @click="chooseArchive">Escolher arquivo…</button>
          <span class="muted small path-preview">{{ archive ?? "nenhum arquivo escolhido" }}</span>
        </div>
      </div>
      <div class="field">
        <span>Descompactar em</span>
        <div class="row">
          <button class="btn" type="button" @click="chooseDest">Escolher pasta…</button>
          <span class="muted small path-preview">{{ destDir ?? "nenhuma pasta escolhida" }}</span>
        </div>
      </div>
    </div>

    <div class="row end">
      <button
        class="btn btn-primary"
        type="button"
        :disabled="!archive || !destDir || !!busy"
        @click="extract"
      >
        Descompactar
      </button>
    </div>

    <div v-if="extracted" class="banner" :class="extracted.skippedCount ? 'banner-warning' : 'banner-ok'">
      <div>
        <strong>Extraído em:</strong> <code>{{ extracted.dest }}</code>
        <p class="small">
          {{ extracted.files }} arquivo(s), {{ extracted.folders }} pasta(s),
          {{ formatBytes(extracted.bytes) }} em {{ (extracted.durationMs / 1000).toFixed(1) }} s.
        </p>
        <details v-if="extracted.skippedCount">
          <summary>{{ extracted.skippedCount }} entrada(s) ignorada(s) por segurança</summary>
          <ul class="small">
            <li v-for="s in extracted.skipped" :key="s.name">
              <code>{{ s.name }}</code>: {{ s.reason }}
            </li>
          </ul>
        </details>
      </div>
      <button class="btn btn-ghost" type="button" @click="extracted = null">Fechar</button>
    </div>
  </section>
</template>
