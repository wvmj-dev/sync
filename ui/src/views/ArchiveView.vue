<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { RouterLink } from "vue-router";
import { api, errorMessage, onArchiveProgress, pickArchive, pickFiles, pickFolder } from "../api";
import type { ArchiveFormat, ArchiveProgress, CompressResult, ExtractResult } from "../api/types";
import Icon from "../components/Icon.vue";
import Notice from "../components/Notice.vue";
import { archiveFormats as formats, formatBytes } from "../utils/format";

// Compactar
const inputs = ref<string[]>([]);
const format = ref<ArchiveFormat>("zip");
const name = ref("");
const outputDir = ref<string | null>(null);
const includeSensitive = ref(false);
/** RN-70: só com a chave ativa e em TAR.ZST (RN-63). */
const encrypt = ref(false);
const keyActive = ref(false);
watch(encrypt, (on) => {
  if (on) format.value = "tar_zst";
});
watch(format, (f) => {
  if (f !== "tar_zst") encrypt.value = false;
});
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
  try {
    keyActive.value = (await api.encryptionStatus()).active !== null;
  } catch {
    keyActive.value = false;
  }
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
      encrypt: encrypt.value,
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
    <div>
      <h1>Compactar</h1>
      <p class="page-desc">
        Junte pastas e arquivos num .zip ou .tar, ou descompacte um que você recebeu. Nada
        existente é sobrescrito.
      </p>
    </div>
  </header>

  <div class="stack">
    <Notice v-if="error" kind="danger" dismissible @close="error = null">{{ error }}</Notice>

    <section v-if="busy" class="boxed pad run" aria-live="polite">
      <div class="run-head">
        <strong>{{ busy === "compress" ? "Compactando…" : "Descompactando…" }}</strong>
        <button class="btn btn-flat btn-small" type="button" @click="api.cancelArchive()">
          <Icon name="stop" :size="14" />
          Cancelar
        </button>
      </div>
      <div class="progress" role="progressbar">
        <div :style="{ width: `${percent ?? 100}%` }" :class="{ indeterminate: percent === null }"></div>
      </div>
      <p v-if="progress" class="muted small num">
        {{ progress.filesDone }}<template v-if="progress.filesTotal"> de {{ progress.filesTotal }}</template>
        arquivo(s), {{ formatBytes(progress.bytesDone) }}<template v-if="progress.bytesTotal">
          de {{ formatBytes(progress.bytesTotal) }}</template>
      </p>
      <p v-if="progress?.current" class="path ellipsis">{{ progress.current }}</p>
    </section>
  </div>

  <section class="section">
    <div class="section-head"><h2>Compactar pastas e arquivos</h2></div>
    <div class="boxed pad form">
      <div class="group">
        <span class="group-title">O que compactar</span>
        <ul v-if="inputs.length" class="inputs">
          <li v-for="(p, i) in inputs" :key="p">
            <Icon name="folder" class="muted" :size="16" />
            <span class="path">{{ p }}</span>
            <button
              class="btn btn-flat btn-small btn-icon"
              type="button"
              :aria-label="`Remover ${p}`"
              @click="inputs.splice(i, 1)"
            >
              <Icon name="close" :size="16" />
            </button>
          </li>
        </ul>
        <div class="row">
          <button class="btn btn-small" type="button" @click="addFolder">
            <Icon name="plus" :size="16" />
            Pasta
          </button>
          <button class="btn btn-small" type="button" @click="addFiles">
            <Icon name="plus" :size="16" />
            Arquivos
          </button>
        </div>
      </div>

      <fieldset class="group">
        <legend>Formato</legend>
        <div class="formats">
          <label v-for="f in formats" :key="f.value" class="format" :class="{ on: format === f.value }">
            <input v-model="format" type="radio" :value="f.value" class="visually-hidden" />
            <span class="format-name">{{ f.label }}</span>
            <span class="format-hint">{{ f.hint }}</span>
          </label>
        </div>
      </fieldset>

      <div class="fields">
        <label class="field">
          <span class="label">Nome do arquivo (opcional)</span>
          <input v-model="name" maxlength="200" placeholder="Padrão: nome da pasta" />
        </label>
        <div class="field">
          <span class="label">Salvar em</span>
          <div class="row">
            <button class="btn btn-small" type="button" @click="chooseOutput">Escolher pasta…</button>
            <span class="path ellipsis">{{ outputDir ?? "" }}</span>
          </div>
        </div>
      </div>

      <label class="field switch-field">
        <span>
          <span class="label">Incluir arquivos sensíveis</span>
          <span class="hint"><code>.env</code>, chaves, <code>.ssh</code>. Não recomendado se for compartilhar.</span>
        </span>
        <input v-model="includeSensitive" class="switch" type="checkbox" role="switch" />
      </label>

      <label class="field switch-field">
        <span>
          <span class="label">Criptografar</span>
          <span v-if="keyActive" class="hint">Gera <code>.tar.zst.age</code>; para abrir, é preciso a chave.</span>
          <span v-else class="hint">
            Crie a chave antes em <RouterLink to="/preferencias">Preferências → Criptografia</RouterLink>.
          </span>
        </span>
        <input v-model="encrypt" class="switch" type="checkbox" role="switch" :disabled="!keyActive" />
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

      <Notice v-if="compressed" kind="ok" dismissible @close="compressed = null">
        <p><strong>Criado:</strong> <span class="path">{{ compressed.path }}</span></p>
        <p class="small num">
          {{ compressed.files }} arquivo(s), {{ formatBytes(compressed.bytesIn) }} viraram
          {{ formatBytes(compressed.bytesOut) }}<template v-if="ratio > 0"> ({{ ratio }}% menor)</template>,
          em {{ (compressed.durationMs / 1000).toFixed(1) }} s.
        </p>
        <details v-if="compressed.warnings.length">
          <summary>{{ compressed.warnings.length }} aviso(s)</summary>
          <ul class="small">
            <li v-for="w in compressed.warnings" :key="w">{{ w }}</li>
          </ul>
        </details>
      </Notice>
    </div>
  </section>

  <section class="section">
    <div class="section-head">
      <div>
        <h2>Descompactar um arquivo</h2>
        <p class="section-desc">
          .zip, .tar, .tar.gz, .tar.zst e .tar.zst.age (com a chave). O conteúdo vai sempre para uma pasta nova; entradas
          perigosas de arquivos baixados da internet são ignoradas.
        </p>
      </div>
    </div>
    <div class="boxed pad form">
      <div class="fields">
        <div class="field">
          <span class="label">Arquivo compactado</span>
          <div class="row">
            <button class="btn btn-small" type="button" @click="chooseArchive">Escolher arquivo…</button>
            <span class="path ellipsis">{{ archive ?? "" }}</span>
          </div>
        </div>
        <div class="field">
          <span class="label">Descompactar em</span>
          <div class="row">
            <button class="btn btn-small" type="button" @click="chooseDest">Escolher pasta…</button>
            <span class="path ellipsis">{{ destDir ?? "" }}</span>
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

      <Notice
        v-if="extracted"
        :kind="extracted.skippedCount ? 'warn' : 'ok'"
        dismissible
        @close="extracted = null"
      >
        <p><strong>Extraído em</strong> <span class="path">{{ extracted.dest }}</span></p>
        <p class="small num">
          {{ extracted.files }} arquivo(s), {{ extracted.folders }} pasta(s),
          {{ formatBytes(extracted.bytes) }}, em {{ (extracted.durationMs / 1000).toFixed(1) }} s.
        </p>
        <details v-if="extracted.skippedCount">
          <summary>{{ extracted.skippedCount }} entrada(s) ignorada(s) por segurança</summary>
          <ul class="small">
            <li v-for="s in extracted.skipped" :key="s.name">
              <span class="path">{{ s.name }}</span>: {{ s.reason }}
            </li>
          </ul>
        </details>
      </Notice>
    </div>
  </section>
</template>

<style scoped>
.form {
  gap: 22px;
}

.inputs {
  display: grid;
  gap: 2px;
  margin: 0;
  padding: 0;
  list-style: none;
}

.inputs li {
  display: flex;
  align-items: center;
  gap: 10px;
}

.inputs .path {
  flex: 1;
  min-width: 0;
}

.formats {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
  gap: 8px;
}

.format {
  display: grid;
  gap: 2px;
  padding: 10px 14px;
  border: 1px solid var(--line);
  border-radius: var(--radius);
  cursor: pointer;
}

.format:hover {
  background: var(--fill);
}

.format.on {
  border-color: var(--accent);
  background: color-mix(in srgb, var(--accent) 10%, transparent);
}

.format:focus-within {
  outline: 2px solid var(--accent);
  outline-offset: 2px;
}

.format-name {
  font-weight: 600;
}

.format-hint {
  color: var(--fg-2);
  font-size: 0.85rem;
}

.switch-field {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
}

.switch-field > span {
  display: grid;
  gap: 2px;
}

.run {
  display: grid;
  gap: 8px;
}

.run-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
}
</style>
