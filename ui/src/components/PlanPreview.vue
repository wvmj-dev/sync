<script setup lang="ts">
import { computed } from "vue";
import type { PlanPreview } from "../api/types";
import { actionLabel, formatBytes, formatLabel } from "../utils/format";

const props = defineProps<{ preview: PlanPreview }>();

const VISIBLE = 200;
const visible = computed(() => props.preview.actions.slice(0, VISIBLE));
const hidden = computed(() => props.preview.totalActions - visible.value.length);

const stats = computed(() => {
  const s = props.preview.stats;
  return [
    { label: "Pastas a criar", value: s.foldersToCreate },
    { label: "Enviar", value: `${s.uploads} (${formatBytes(s.bytesToSend)})` },
    { label: "Atualizar", value: s.updates },
    { label: "Conflitos", value: s.conflicts },
    { label: "Lixeira", value: s.trashFiles + s.trashFolders },
    { label: "Já idênticos", value: s.adopted },
    { label: "Ignorados", value: s.skipped },
  ];
});
</script>

<template>
  <div class="preview">
    <div v-if="preview.verdict.verdict === 'proceed'" class="banner banner-ok">
      Pronto para executar.
    </div>
    <div v-else-if="preview.verdict.verdict === 'needs_confirmation'" class="banner banner-warning">
      <div>
        <strong>Exige confirmação:</strong>
        <ul>
          <li v-for="r in preview.verdict.reasons" :key="r">{{ r }}</li>
        </ul>
        <p class="small">Revise o plano abaixo; ao clicar em <strong>Executar</strong>, o app pede a sua confirmação.</p>
      </div>
    </div>
    <div v-else class="banner banner-danger">
      <div>
        <strong>Bloqueado:</strong>
        <ul>
          <li v-for="r in preview.verdict.reasons" :key="r">{{ r }}</li>
        </ul>
      </div>
    </div>

    <template v-if="preview.receive">
      <p v-if="!preview.receive.name" class="muted">Nenhuma versão no destino ainda.</p>
      <p v-else-if="preview.receive.alreadyReceived" class="muted">
        A versão mais recente (<code>{{ preview.receive.name }}</code>) já foi recebida — nada a
        fazer.
      </p>
      <dl v-else class="stats">
        <div>
          <dt>Versão mais recente</dt>
          <dd><code>{{ preview.receive.name }}</code></dd>
        </div>
        <div v-if="preview.receive.size != null">
          <dt>Tamanho</dt>
          <dd>{{ formatBytes(preview.receive.size) }}</dd>
        </div>
        <div>
          <dt>Descompactar em</dt>
          <dd><code>{{ preview.receive.target }}</code> (pasta nova)</dd>
        </div>
      </dl>
      <p class="muted small">Calculado em {{ preview.durationMs }} ms · recebimento</p>
    </template>

    <template v-else-if="preview.archive">
      <p v-if="preview.archive.unchanged" class="muted">
        Nada mudou desde a última versão — nada será enviado
        ({{ preview.archive.versions }} versão(ões) no destino).
      </p>
      <template v-else>
        <dl class="stats">
          <div>
            <dt>Nova versão</dt>
            <dd><code>{{ preview.archive.name }}</code></dd>
          </div>
          <div>
            <dt>Conteúdo</dt>
            <dd>
              {{ preview.archive.files }} arquivo(s), {{ preview.archive.folders }} pasta(s) ·
              {{ formatBytes(preview.archive.bytes) }} antes de compactar
            </dd>
          </div>
          <div>
            <dt>Versões no destino</dt>
            <dd>{{ preview.archive.versions }}</dd>
          </div>
          <div>
            <dt>Lixeira</dt>
            <dd>{{ preview.archive.toTrash.length }} versão(ões) antiga(s)</dd>
          </div>
        </dl>
        <details v-if="preview.archive.toTrash.length">
          <summary>Versões que vão para a lixeira depois do envio</summary>
          <ul class="small">
            <li v-for="n in preview.archive.toTrash" :key="n"><code>{{ n }}</code></li>
          </ul>
        </details>
      </template>
      <p class="muted small">
        Calculado em {{ preview.durationMs }} ms · envio compactado
        ({{ formatLabel(preview.archive.format) }})
      </p>
    </template>

    <template v-else>
      <dl class="stats">
        <div v-for="s in stats" :key="s.label">
          <dt>{{ s.label }}</dt>
          <dd>{{ s.value }}</dd>
        </div>
      </dl>
      <p class="muted small">
        Calculado em {{ preview.durationMs }} ms · {{ preview.hashedFiles }} arquivo(s) com hash
        recalculado · modo {{ preview.mode === "upload" ? "envio" : "espelho" }}
      </p>

      <table v-if="visible.length" class="table">
        <thead>
          <tr>
            <th>Ação</th>
            <th>Caminho</th>
            <th>Detalhe</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="(a, i) in visible" :key="i">
            <td><span class="tag" :class="`tag-${a.kind}`">{{ actionLabel[a.kind] }}</span></td>
            <td><code>{{ a.path }}</code></td>
            <td class="muted">{{ a.detail ?? "" }}</td>
          </tr>
        </tbody>
      </table>
      <p v-else class="muted">Nada a fazer — tudo sincronizado.</p>
      <p v-if="hidden > 0" class="muted small">… e mais {{ hidden }} ação(ões).</p>
    </template>

    <details v-if="preview.warnings.length">
      <summary>{{ preview.warnings.length }} aviso(s)</summary>
      <ul class="small">
        <li v-for="w in preview.warnings" :key="w">{{ w }}</li>
      </ul>
    </details>
  </div>
</template>
