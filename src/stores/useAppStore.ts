import { create } from "zustand";
import { listen } from "@tauri-apps/api/event";
import { api } from "../lib/api";
import type {
  Anime,
  AnimeDetail,
  BufferConfig,
  BufferStatus,
  CatalogFilter,
  CatalogPage,
  PlayerProgressEvent,
  PlayerState,
  ProviderInfo,
  Tag,
  WatchHistoryEntry,
} from "../lib/types";

export type View = "home" | "catalog" | "search" | "favorites" | "history" | "detail" | "settings";

export const DEFAULT_FILTER: CatalogFilter = {
  genero: null,
  demografia: null,
  temporada: null,
  tipo: null,
  estado: null,
  anio: null,
  orden: null,
};

interface AppStore {
  view: View;
  setView: (v: View) => void;
  prevView: View;
  goBack: () => void;

  detail: AnimeDetail | null;
  detailLoading: boolean;
  detailError: string | null;
  openDetail: (slug: string) => Promise<void>;
  closeDetail: () => void;

  recent: Anime[];
  recommended: Anime[];
  homeLoading: boolean;
  loadHome: () => Promise<void>;

  tags: Tag[];
  loadTags: () => Promise<void>;

  filter: CatalogFilter;
  setFilter: (patch: Partial<CatalogFilter>) => void;
  resetFilter: () => void;
  catalog: CatalogPage | null;
  catalogPage: number;
  catalogLoading: boolean;
  loadCatalog: (page?: number) => Promise<void>;

  searchQuery: string;
  setSearchQuery: (q: string) => void;
  searchResults: Anime[];
  searchLoading: boolean;
  runSearch: (query: string) => Promise<void>;

  favorites: Anime[];
  favoritesLoading: boolean;
  loadFavorites: () => Promise<void>;
  toggleFavorite: (slug: string) => Promise<boolean>;

  history: WatchHistoryEntry[];
  historyLoading: boolean;
  loadHistory: () => Promise<void>;
  clearHistory: () => Promise<void>;

  player: PlayerState;
  playerVisible: boolean;
  initPlayer: () => Promise<void>;
  refreshPlayer: () => Promise<void>;

  buffer: BufferStatus;
  bufferConfig: BufferConfig | null;
  bufferLoading: boolean;
  loadBuffer: () => Promise<void>;
  initBuffer: () => Promise<void>;

  provider: ProviderInfo | null;
  providerLoading: boolean;
  loadProvider: () => Promise<void>;
  setProvider: (key: string) => Promise<void>;
}

export const useAppStore = create<AppStore>((set, get) => ({
  view: "home",
  setView: (v) => set({ view: v }),
  prevView: "home",
  goBack: () => {
    const prev = get().prevView;
    const wasDetail = get().view === "detail";
    if (wasDetail) get().closeDetail();
    if (prev === "home") get().loadHome();
    if (prev === "favorites") get().loadFavorites();
    if (prev === "history") get().loadHistory();
    set({ view: prev });
  },

  detail: null,
  detailLoading: false,
  detailError: null,
  openDetail: async (slug) => {
    set({ prevView: get().view, detailLoading: true, detailError: null, view: "detail" });
    try {
      const detail = await api.getAnimeDetail(slug);
      set({ detail, detailLoading: false });
    } catch (e) {
      set({ detailError: (e as Error).message, detailLoading: false });
    }
  },
  closeDetail: () => set({ detail: null, detailError: null }),

  recent: [],
  recommended: [],
  homeLoading: false,
  loadHome: async () => {
    if (get().homeLoading) return;
    set({ homeLoading: true });
    try {
      const [recent, recommended] = await Promise.all([api.getRecent(), api.getRecommended()]);
      set({ recent, recommended, homeLoading: false });
    } catch (e) {
      console.error(e);
      set({ homeLoading: false });
    }
  },

  tags: [],
  loadTags: async () => {
    try {
      const tags = await api.getTags();
      set({ tags });
    } catch (e) {
      console.error(e);
    }
  },

  filter: { ...DEFAULT_FILTER },
  setFilter: (patch) => {
    set({ filter: { ...get().filter, ...patch } });
    get().loadCatalog(1);
  },
  resetFilter: () => {
    set({ filter: { ...DEFAULT_FILTER } });
    get().loadCatalog(1);
  },
  catalog: null,
  catalogPage: 1,
  catalogLoading: false,
  loadCatalog: async (page = get().catalogPage) => {
    set({ catalogLoading: true, catalogPage: page });
    try {
      const res = await api.getCatalog(get().filter, page);
      set({ catalog: res, catalogLoading: false });
    } catch (e) {
      console.error(e);
      set({ catalogLoading: false });
    }
  },

  searchQuery: "",
  setSearchQuery: (q) => set({ searchQuery: q }),
  searchResults: [],
  searchLoading: false,
  runSearch: async (query) => {
    set({ searchLoading: true });
    try {
      const res = await api.searchAnime(query);
      set({ searchResults: res, searchLoading: false });
    } catch (e) {
      console.error(e);
      set({ searchResults: [], searchLoading: false });
    }
  },

  favorites: [],
  favoritesLoading: false,
  loadFavorites: async () => {
    set({ favoritesLoading: true });
    try {
      const entries = await api.listFavorites();
      set({ favorites: entries.map((e) => e.anime), favoritesLoading: false });
    } catch (e) {
      console.error(e);
      set({ favoritesLoading: false });
    }
  },
  toggleFavorite: async (slug) => {
    try {
      const isFav = await api.isFavorite(slug);
      if (isFav) {
        await api.removeFavorite(slug);
      } else {
        await api.addFavorite(slug);
      }
      await get().loadFavorites();
      return !isFav;
    } catch (e) {
      console.error(e);
      return get().favorites.some((a) => a.slug === slug);
    }
  },

  history: [],
  historyLoading: false,
  loadHistory: async () => {
    set({ historyLoading: true });
    try {
      const res = await api.continueWatching();
      set({ history: res, historyLoading: false });
    } catch (e) {
      console.error(e);
      set({ historyLoading: false });
    }
  },
  clearHistory: async () => {
    try {
      await api.clearHistory();
      set({ history: [] });
    } catch (e) {
      console.error(e);
    }
  },

  player: {
    session_id: 0,
    phase: "idle",
    loaded: false,
    playing: false,
    position: 0,
    duration: 0,
    speed: 1,
    volume: 100,
    muted: false,
    fullscreen: false,
    title: null,
    slug: null,
    number: 0,
    error: null,
  },
  playerVisible: false,
  initPlayer: async () => {
    await get().refreshPlayer();
    const onState = await listen<PlayerState>("player://state", (e) => {
      set({ player: e.payload });
    });
    const onProgress = await listen<PlayerProgressEvent>("player://progress", (e) => {
      // Segunda barrera de protección: descartar eventos de sesiones antiguas.
      if (e.payload.session_id < get().player.session_id) return;
      get().refreshPlayer();
    });
    const onEnd = await listen<PlayerProgressEvent>("player://end", (e) => {
      // Un `player://end` de una reproducción anterior no debe mostrar el
      // episodio actual como finalizado.
      if (e.payload.session_id < get().player.session_id) return;
      const p = { ...get().player, playing: false };
      set({ player: p });
      get().loadHistory();
    });
    const onError = await listen<string>("player://error", (e) => {
      set({ player: { ...get().player, error: e.payload } });
    });
    window.__unlisteners = [onState, onProgress, onEnd, onError];
  },
  refreshPlayer: async () => {
    try {
      const state = await api.playerGetState();
      set({ player: state, playerVisible: state.loaded });
    } catch {
      // sin reproductor disponible
    }
  },

  buffer: {
    enabled: false,
    paused: false,
    pause_reasons: [],
    cache_bytes: 0,
    cache_limit_bytes: 0,
    current_episode: null,
    queue: [],
  },
  bufferConfig: null,
  bufferLoading: false,
  loadBuffer: async () => {
    set({ bufferLoading: true });
    try {
      const [status, config] = await Promise.all([api.bufferGetStatus(), api.bufferGetConfig()]);
      set({ buffer: status, bufferConfig: config, bufferLoading: false });
    } catch (e) {
      console.error(e);
      set({ bufferLoading: false });
    }
  },
  initBuffer: async () => {
    await get().loadBuffer();
    const onStatus = await listen<BufferStatus>("buffer://status", (e) => {
      set({ buffer: e.payload });
    });
    window.__unlisteners = [...(window.__unlisteners ?? []), onStatus];
  },

  provider: null,
  providerLoading: false,
  loadProvider: async () => {
    try {
      const info = await api.getProvider();
      set({ provider: info });
    } catch (e) {
      console.error(e);
    }
  },
  setProvider: async (key) => {
    if (get().providerLoading) return;
    set({ providerLoading: true });
    try {
      const info = await api.setProvider(key);
      set({ provider: info });
      // El contenido cacheado pertenece a la fuente anterior: recargar la UI.
      set({
        recent: [],
        recommended: [],
        tags: [],
        catalog: null,
        detail: null,
        searchResults: [],
      });
      get().loadHome();
      get().loadTags();
      if (get().view === "catalog") get().loadCatalog(1);
    } catch (e) {
      console.error(e);
    } finally {
      set({ providerLoading: false });
    }
  },
}));
