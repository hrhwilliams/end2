// <reference types="vite/client" />

interface ImportMetaEnv {
  // Both are pinned at build time and never fetched from the provider.
  readonly VITE_VRF_PUBLIC_KEY?: string;
  // Comma-separated CometBFT RPC base URLs. Unset keeps the provider-only key lookup.
  readonly VITE_COMET_RPC_URLS?: string;
}

declare module "*.vue" {
  import type { DefineComponent } from "vue";
  // eslint-disable-next-line @typescript-eslint/no-explicit-any, @typescript-eslint/ban-types
  const component: DefineComponent<{}, {}, any>;
  export default component;
}
