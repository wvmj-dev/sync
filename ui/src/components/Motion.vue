<script setup lang="ts">
// As transições do app (`slide`, `swap`) — use sempre este componente, nunca `<Transition>`
// direto. No WebKitGTK, se o compositor chega sozinho ao fim de uma animação, ele mostra por
// 1 a 3 quadros o valor antigo da camada: a tela que saía volta inteira, a que entrou pisca.
// Por isso a animação do CSS dura mais que o movimento (fica parada no valor final) e quem
// a encerra é o Vue, pela duração dos tokens `--motion-in`/`--motion-out` (ver main.css).
defineProps<{ name: "slide" | "swap"; mode?: "out-in" }>();

/** Duração de um token do main.css em ms ("180ms" → 180; 0 se não houver). */
function tokenMs(token: string): number {
  const value = getComputedStyle(document.documentElement).getPropertyValue(token).trim();
  const n = Number.parseFloat(value);
  if (!Number.isFinite(n)) return 0;
  return value.endsWith("ms") ? n : n * 1000;
}

// Lido a cada troca: com `prefers-reduced-motion` os tokens valem 0 e a troca é imediata.
const duration = () => ({ enter: tokenMs("--motion-in"), leave: tokenMs("--motion-out") });
</script>

<template>
  <Transition :name="name" :mode="mode" :duration="duration()">
    <slot />
  </Transition>
</template>
