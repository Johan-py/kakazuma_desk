export interface Anime {
  id: number;
  slug: string;
  name: string;
  synopsis: string | null;
  season: string | null;
  status: string | null;
  cover_image: string | null;
  total_episodes: number | null;
  anime_type: string | null;
  url: string;
  genres: string[];
}

export interface Episode {
  id: number;
  anime_id: number;
  number: number;
  title: string | null;
  video_url: string | null;
  duration: number | null;
}

export interface Tag {
  id: number;
  name: string;
  description: string | null;
}

export interface Subtitle {
  lang: string;
  url: string;
}

export interface VideoSource {
  url: string;
  quality: string | null;
  subtitles: Subtitle[];
}

export interface AnimeDetail {
  anime: Anime;
  episodes: Episode[];
  tags: Tag[];
}

export interface WatchHistoryEntry {
  id: number;
  anime: Anime;
  episode: Episode | null;
  playback_position: number;
  duration: number;
  date_first_view: number;
  date_last_view: number;
}

export interface FavoriteEntry {
  id: number;
  anime: Anime;
  date_added: number;
}

export interface CatalogFilter {
  genero: string | null;
  demografia: string | null;
  temporada: string | null;
  tipo: string | null;
  estado: string | null;
  anio: number | null;
  orden: string | null;
}

export interface CatalogPage {
  items: Anime[];
  page: number;
  total: number;
  per_page: number;
  last_page: number;
}

export interface PlayerState {
  loaded: boolean;
  playing: boolean;
  position: number;
  duration: number;
  speed: number;
  volume: number;
  muted: boolean;
  fullscreen: boolean;
  title: string | null;
  slug: string | null;
  number: number;
  error: string | null;
}

export interface PlayerProgressEvent {
  slug: string;
  number: number;
  position: number;
  duration: number;
}
