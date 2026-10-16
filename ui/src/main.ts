import { createApp } from "vue";
import { createPinia } from "pinia";
import App from "./App.vue";
import { router } from "./router";
import "./styles/main.css";
import { followSystemTheme } from "./theme";

void followSystemTheme();
createApp(App).use(createPinia()).use(router).mount("#app");
