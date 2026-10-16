<script setup lang="ts">
import { computed } from "vue";
import type { PlanPreview } from "../api/types";
import { actionChip, actionLabel, formatBytes, formatLabel } from "../utils/format";
import Notice from "./Notice.vue";

const props = defineProps<{ preview: PlanPreview }>();

const VISIBLE = 200;
const visible = computed(() => props.preview.actions.slice(0, VISIBLE));
const hidden = computed(() => props.preview.totalActions - visible.value.length);

/** Só os números que importam: zeros ficam de fora. */
const stats = computed(() => {
  const s = props.preview.stats;
  return [
    { label: "Enviar", value: s.uploads, extra: s.uploads ? formatBytes(s.bytesToSend) : "" },
    { label: "Atualizar", value: s.updates, extra: "" },
    { label: "Pastas a criar", value: s.foldersToCreate, extra: "" },
    { label: "Conflitos", value: s.conflicts, extra: "" },
    { label: "Para a lixeira", value: s.trashFiles + s.trashFolders, extra: "" },
    { label: "Já idênticos", value: s.adopted, extra: "" },
    { label: "Ignorados", value: s.skipped, extra: "" },
  ].filter((x) => x.value > 0);
});

const verdict = computed(() => props.preview.verdict);
</script>

<template>
  <div class="preview">
    <Notice v-if="verdict.verdict === 'proceed'" kind="ok">Pronto para executar.</Notice>
    <Notice v-else-if="verdict.verdict === 'needs_confirmation'" kind="warn">
      <p><strong>Ao executar, o app vai pedir a sua confirmação:</strong></p>
      <ul>
        <li v-for="r in verdict.reasons" :key="r">{{ r }}</li>
      </ul>
    </Notice>
    <Notice v-else kind="danger">
      <p><strong>A execução está bloqueada:</strong></p>
      <ul>
        <li v-for="r in verdict.reasons" :key="r">{{ r }}</li>
      </ul>
    </Notice>

    <!-- Tarefa que recebe -->
    <template v-if="preview.receive">
      <p v-if="!preview.receive.name" class="muted">Ainda não há nenhuma versão no destino.</p>
      <p v-else-if="preview.receive.alreadyReceived" class="muted">
        A versão mais recente ({{ preview.receive.name }}) já foi recebida; não há nada a fazer.
      </p>
      <dl v-else class="facts">
        <div>
          <dt>Versão mais recente</dt>
          <dd class="path">{{ preview.receive.name }}</dd>
        </div>
        <div v-if="preview.receive.size != null">
          <dt>Tamanho</dt>
          <dd class="num">{{ formatBytes(preview.receive.size) }}</dd>
        </div>
        <div class="wide">
          <dt>Será descompactada numa pasta nova em</dt>
          <dd class="path">{{ preview.receive.target }}</dd>
        </div>
      </dl>
    </template>

    <!-- Tarefa compactada -->
    <template v-else-if="preview.archive">
      <p v-if="preview.archive.unchanged" class="muted">
        Nada mudou desde a última versão; nada será enviado. Há
        {{ preview.archive.versions }} versão(ões) no destino.
      </p>
      <template v-else>
        <dl class="facts">
          <div class="wide">
            <dt>Nova versão</dt>
            <dd class="path">{{ preview.archive.name }}</dd>
          </div>
          <div>
            <dt>Conteúdo</dt>
            <dd class="num">
              {{ preview.archive.files }} arquivo(s), {{ formatBytes(preview.archive.bytes) }}
            </dd>
          </div>
          <div>
            <dt>Formato</dt>
            <dd>{{ formatLabel(preview.archive.format) }}</dd>
          </div>
          <div>
            <dt>Versões no destino</dt>
            <dd class="num">{{ preview.archive.versions }}</dd>
          </div>
          <div>
            <dt>Antigas para a lixeira</dt>
            <dd class="num">{{ preview.archive.toTrash.length }}</dd>
          </div>
        </dl>
        <details v-if="preview.archive.toTrash.length">
          <summary>Quais versões vão para a lixeira depois do envio</summary>
          <ul class="small">
            <li v-for="n in preview.archive.toTrash" :key="n" class="path">{{ n }}</li>
          </ul>
        </details>
      </template>
    </template>

    <!-- Arquivo por arquivo -->
    <template v-else>
      <dl v-if="stats.length" class="facts">
        <div v-for="s in stats" :key="s.label">
          <dt>{{ s.label }}</dt>
          <dd class="stat num">
            {{ s.value }}
            <span v-if="s.extra" class="block muted small">{{ s.extra }}</span>
          </dd>
        </div>
      </dl>
      <p v-else class="muted">Nada a fazer: o destino já está igual.</p>

      <div v-if="visible.length" class="table-wrap">
        <table class="table">
          <thead>
            <tr>
              <th>Ação</th>
              <th>Caminho</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="(a, i) in visible" :key="i">
              <td class="action"><span class="chip" :class="actionChip[a.kind]">{{ actionLabel[a.kind] }}</span></td>
              <td>
                <span class="path block">{{ a.path }}</span>
                <span v-if="a.detail" class="muted small">{{ a.detail }}</span>
              </td>
            </tr>
          </tbody>
        </table>
      </div>
      <p v-if="hidden > 0" class="muted small">E mais {{ hidden }} ação(ões).</p>
    </template>

    <details v-if="preview.warnings.length">
      <summary>{{ preview.warnings.length }} aviso(s)</summary>
      <ul class="small">
        <li v-for="w in preview.warnings" :key="w">{{ w }}</li>
      </ul>
    </details>

    <p class="muted small">
      Calculada em {{ preview.durationMs }} ms<template v-if="preview.hashedFiles">;
        {{ preview.hashedFiles }} arquivo(s) conferidos por conteúdo</template>.
    </p>
  </div>
</template>

<style scoped>
.preview {
  display: grid;
  gap: 16px;
}

.stat {
  font-size: 1.05rem;
  font-weight: 600;
}

.stat .small {
  font-weight: 400;
}

.action {
  width: 1%;
  white-space: nowrap;
}

.path.block {
  color: var(--fg);
}
</style>
