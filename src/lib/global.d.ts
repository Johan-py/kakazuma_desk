import type { UnlistenFn } from "@tauri-apps/api/event";

declare global {
  interface Window {
    __unlisteners?: UnlistenFn[];
  }
}

export {};
