<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { api, errorMessage } from "../api";
import type { NotifyPolicy, Preferences } from "../api/types";
import Notice from "../components/Notice.vue";

const saved = ref<Preferences | null>(null);
const notify = ref<NotifyPolicy>("failures");
const parallel = ref(4);
/** Reserva mostrada em GiB; guardada em MiB. */
const reserveGib = ref(1);
const saving = ref(false);
const notice = ref<{ kind: "ok" | "danger"; text: string } | null>(null);

const notifyOptions: { value: NotifyPolicy; title: string; desc: string }[] = [
  {
    value: "failures",
    title: "Só quando algo precisa de atenção (recomendado)",
    desc: "Falhou, foi bloqueada, terminou com falhas, aguarda a sua confirmação, a conta precisa ser reconectada ou ficou sem rede 3 vezes seguidas.",
  },
  { value: "always", title: "Sempre", desc: "Também avisa quando a execução dá certo." },
  { value: "never", title: "Nunca", desc: "Confira o resultado no app, em Tarefas ou na Visão geral." },
];

function apply(p: Preferences) {
  saved.value = p;
  notify.value = p.notify;
  parallel.value = p.maxParallelTransfers;
  reserveGib.value = Math.round((p.minFreeSpaceMib / 1024) * 100) / 100;
}

const draft = computed<Preferences>(() => ({
  notify: notify.value,
  maxParallelTransfers: parallel.value,
  minFreeSpaceMib: Math.round(reserveGib.value * 1024),
}));

const valid = computed(
  () =>
    Number.isInteger(parallel.value) &&
    parallel.value >= 1 &&
    parallel.value <= 8 &&
    reserveGib.value >= 0 &&
    reserveGib.value <= 1024,
);
// Campo a campo: a ordem das chaves que vem do Rust não importa.
const changed = computed(() => {
  const s = saved.value;
  const d = draft.value;
  return (
    !!s &&
    (s.notify !== d.notify ||
      s.maxParallelTransfers !== d.maxParallelTransfers ||
      s.minFreeSpaceMib !== d.minFreeSpaceMib)
  );
});

async function save() {
  saving.value = true;
  notice.value = null;
  try {
    apply(await api.savePreferences(draft.value));
    notice.value = { kind: "ok", text: "Preferências salvas." };
  } catch (e) {
    notice.value = { kind: "danger", text: errorMessage(e) };
  } finally {
    saving.value = false;
  }
}

onMounted(async () => {
  try {
    apply(await api.getPreferences());
  } catch (e) {
    notice.value = { kind: "danger", text: errorMessage(e) };
  }
});
</script>

<template>
  <header class="page-head">
    <div>
      <h1>Preferências</h1>
      <p class="page-desc">Quando o Syncly avisa e quanto ele pode usar da conta e da rede.</p>
    </div>
  </header>

  <form class="stack" @submit.prevent="save">
    <section class="section first">
      <div class="section-head">
        <div>
          <h2>Notificações</h2>
          <p class="section-desc">
            Valem para as execuções agendadas. Ao executar pelo app, o resultado aparece na tela.
          </p>
        </div>
      </div>
      <fieldset class="group">
        <legend class="visually-hidden">Quando avisar</legend>
        <div class="boxed">
          <label v-for="o in notifyOptions" :key="o.value" class="option">
            <input v-model="notify" type="radio" name="notify" :value="o.value" />
            <span class="option-main">
              <span class="option-title">{{ o.title }}</span>
              <span class="option-desc">{{ o.desc }}</span>
            </span>
          </label>
        </div>
      </fieldset>
    </section>

    <section class="section">
      <div class="section-head"><h2>Transferências</h2></div>
      <div class="boxed pad fields">
        <label class="field">
          <span class="label">Envios ao mesmo tempo</span>
          <input v-model.number="parallel" type="number" min="1" max="8" step="1" required />
          <span class="hint">De 1 a 8. Mais envios terminam antes, mas ocupam mais a rede. Padrão: 4.</span>
        </label>
        <label class="field">
          <span class="label">Espaço que deve sobrar na conta (GiB)</span>
          <input v-model.number="reserveGib" type="number" min="0" max="1024" step="0.5" required />
          <span class="hint">
            Um envio é interrompido antes de deixar menos que isso livre: uma conta Google cheia
            para de receber e-mails no Gmail. Padrão: 1 GiB.
          </span>
        </label>
      </div>
    </section>

    <Notice v-if="notice" :kind="notice.kind" dismissible @close="notice = null">
      {{ notice.text }}
    </Notice>

    <div class="row end">
      <button class="btn btn-flat" type="button" :disabled="!changed || saving" @click="saved && apply(saved)">
        Desfazer
      </button>
      <button class="btn btn-primary" type="submit" :disabled="!changed || !valid || saving">
        Salvar
      </button>
    </div>
  </form>
</template>

<style scoped>
.section.first {
  margin-top: 0;
}
</style>
