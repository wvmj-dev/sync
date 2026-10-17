<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { api, errorMessage, pickFile, pickFolder } from "../api";
import type { EncryptionStatus, NewEncryptionKey, NotifyPolicy, Preferences } from "../api/types";
import Motion from "../components/Motion.vue";
import Notice from "../components/Notice.vue";
import { plural } from "../utils/format";

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

// ---- Criptografia (RN-70 a RN-74). A chave privada nunca chega aqui. ----
const crypto = ref<EncryptionStatus | null>(null);
/** Chave criada e gravada, esperando a confirmação de que o arquivo foi guardado (RN-72). */
const pending = ref<NewEncryptionKey | null>(null);
const keySaved = ref(false);
const cryptoBusy = ref(false);
const cryptoNotice = ref<{ kind: "ok" | "danger"; text: string } | null>(null);
const oldKeys = computed(() => crypto.value?.known.filter((k) => k !== crypto.value?.active) ?? []);

/** "3f9a1c0b7d2e4a65" → "3f9a 1c0b 7d2e 4a65": mais fácil de conferir com o arquivo. */
function groups(fingerprint: string): string {
  return fingerprint.match(/.{1,4}/g)?.join(" ") ?? fingerprint;
}

async function cryptoAction(op: () => Promise<void>) {
  cryptoBusy.value = true;
  cryptoNotice.value = null;
  try {
    await op();
  } catch (e) {
    cryptoNotice.value = { kind: "danger", text: errorMessage(e) };
  } finally {
    cryptoBusy.value = false;
  }
}

async function createKey() {
  const folder = await pickFolder("Onde salvar a chave de recuperação (de preferência um pendrive)");
  if (!folder) return;
  await cryptoAction(async () => {
    pending.value = await api.createEncryptionKey(folder);
    keySaved.value = false;
  });
}

async function activateKey() {
  const key = pending.value;
  if (!key) return;
  await cryptoAction(async () => {
    crypto.value = await api.activateEncryptionKey(key.publicKey, keySaved.value);
    pending.value = null;
    cryptoNotice.value = {
      kind: "ok",
      text: "Criptografia ativa. Ligue-a nas tarefas compactadas em TAR.ZST.",
    };
  });
}

async function importKey() {
  const file = await pickFile("Arquivo de recuperação da chave (syncly-chave-….txt)");
  if (!file) return;
  await cryptoAction(async () => {
    const before = crypto.value?.active ?? null;
    crypto.value = await api.importEncryptionKey(file);
    cryptoNotice.value = {
      kind: "ok",
      text:
        before === null && crypto.value.active
          ? "Chave importada e ativa."
          : "Chave importada: as versões feitas com ela voltam a abrir neste computador.",
    };
  });
}

onMounted(async () => {
  try {
    apply(await api.getPreferences());
    crypto.value = await api.encryptionStatus();
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

    <Motion name="slide">
      <Notice v-if="notice" :kind="notice.kind" dismissible @close="notice = null">
        {{ notice.text }}
      </Notice>
    </Motion>

    <div class="row end">
      <button class="btn btn-flat" type="button" :disabled="!changed || saving" @click="saved && apply(saved)">
        Desfazer
      </button>
      <button class="btn btn-primary" type="submit" :disabled="!changed || !valid || saving">
        Salvar
      </button>
    </div>
  </form>

  <section v-if="crypto" class="section" aria-labelledby="crypto-title">
    <div class="section-head">
      <div>
        <h2 id="crypto-title">Criptografia</h2>
        <p class="section-desc">
          Cifra as versões compactadas no computador, antes do envio: no destino ficam só dados
          ilegíveis. Vale para tarefas e compactações em TAR.ZST.
        </p>
      </div>
    </div>

    <div class="boxed pad">
      <!-- "Chave criada" entra no lugar da situação, e volta ao ativar. -->
      <Motion name="swap" mode="out-in">
      <div v-if="pending" key="pending" class="stack">
        <div>
          <p><strong>Chave criada.</strong> O arquivo de recuperação foi salvo em:</p>
          <p class="path">{{ pending.recoveryFile }}</p>
        </div>
        <Notice kind="warn">
          Sem esse arquivo, as versões criptografadas não podem ser abertas — nem pelo Syncly.
          Copie-o para fora deste computador (pendrive, gerenciador de senhas) antes de ativar.
        </Notice>
        <label class="field switch-field">
          <span class="label">Guardei o arquivo de recuperação fora deste computador</span>
          <input v-model="keySaved" class="switch" type="checkbox" role="switch" />
        </label>
        <div class="row end">
          <button class="btn btn-flat" type="button" :disabled="cryptoBusy" @click="pending = null">
            Agora não
          </button>
          <button class="btn btn-primary" type="button" :disabled="!keySaved || cryptoBusy" @click="activateKey">
            Ativar criptografia
          </button>
        </div>
      </div>

      <div v-else key="status" class="stack">
        <dl class="facts">
          <div>
            <dt>Chave das novas versões</dt>
            <dd v-if="crypto.active" class="num">{{ groups(crypto.active) }}</dd>
            <dd v-else class="muted">Nenhuma — as versões vão sem criptografia</dd>
          </div>
          <div v-if="oldKeys.length">
            <dt>Chaves antigas</dt>
            <dd>{{ plural(oldKeys.length, "chave", "chaves") }} para abrir versões anteriores</dd>
          </div>
        </dl>
        <p v-if="crypto.active" class="muted small">
          Trocar de chave vale para as próximas versões; as anteriores continuam abrindo com a
          chave antiga, que fica guardada neste computador.
        </p>
        <div class="row end">
          <button class="btn btn-flat" type="button" :disabled="cryptoBusy" @click="importKey">
            Importar chave…
          </button>
          <button
            class="btn"
            :class="{ 'btn-primary': !crypto.active }"
            type="button"
            :disabled="cryptoBusy"
            @click="createKey"
          >
            {{ crypto.active ? "Trocar de chave…" : "Criar chave…" }}
          </button>
        </div>
      </div>
      </Motion>
    </div>

    <Motion name="slide">
      <Notice v-if="cryptoNotice" :kind="cryptoNotice.kind" dismissible @close="cryptoNotice = null">
        {{ cryptoNotice.text }}
      </Notice>
    </Motion>
  </section>
</template>

<style scoped>
.section.first {
  margin-top: 0;
}
</style>
