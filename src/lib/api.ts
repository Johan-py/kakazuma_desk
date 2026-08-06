import { invoke } from "@tauri-apps/api/core";
import type {
  Anime,
  AnimeDetail,
  CatalogFilter,
  CatalogPage,
  FavoriteEntry,
  PlayerState,
  Tag,
  VideoSource,
  WatchHistoryEntry,
} from "./types";

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(cmd, args);
  } catch (e) {
    throw new Error(typeof e === "string" ? e : "Error desconocido");
  }
}

export const api = {
  searchAnime: (query: string) => call<Anime[]>("search_anime", { query }),

  getAnimeDetail: (slug: string) => call<AnimeDetail>("get_anime_detail", { slug }),

  getCatalog: (filter: CatalogFilter, page: number) =>
    call<CatalogPage>("get_catalog", { filter, page }),

  getTags: () => call<Tag[]>("get_tags"),

  getRecent: () => call<Anime[]>("get_recent"),

  getRecommended: () => call<Anime[]>("get_recommended"),

  resolveVideo: (slug: string, number: number) =>
    call<VideoSource>("resolve_video", { slug, number }),

  listFavorites: () => call<FavoriteEntry[]>("list_favorites"),

  addFavorite: (slug: string) => call<boolean>("add_favorite", { slug }),

  removeFavorite: (slug: string) => call<void>("remove_favorite", { slug }),

  isFavorite: (slug: string) => call<boolean>("is_favorite", { slug }),

  continueWatching: () => call<WatchHistoryEntry[]>("continue_watching"),

  saveProgress: (slug: string, episodeNumber: number | null, position: number, duration: number) =>
    call<void>("save_progress", { slug, episodeNumber, position, duration }),

  clearHistory: () => call<void>("clear_history"),

  playEpisode: (slug: string, number: number, title: string, start: number) =>
    call<VideoSource>("play_episode", { slug, number, title, start }),

  playerPause: () => call<void>("player_pause"),
  playerResume: () => call<void>("player_resume"),
  playerTogglePause: () => call<void>("player_toggle_pause"),
  playerSeek: (position: number) => call<void>("player_seek", { position }),
  playerSetSpeed: (speed: number) => call<void>("player_set_speed", { speed }),
  playerSetVolume: (volume: number) => call<void>("player_set_volume", { volume }),
  playerToggleMute: () => call<void>("player_toggle_mute"),
  playerFullscreen: (enabled: boolean) => call<void>("player_fullscreen", { enabled }),
  playerStop: () => call<void>("player_stop"),
  playerGetState: () => call<PlayerState>("player_get_state"),
};
