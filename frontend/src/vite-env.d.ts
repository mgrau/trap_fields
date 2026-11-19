/// <reference types="vite/client" />

interface ImportMetaEnv {
  /** "1" in builds for static hosting without a server (see src/isolation.ts) */
  readonly VITE_STATIC?: string;
}
