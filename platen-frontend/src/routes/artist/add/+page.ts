import { API_URL, PAGE_SIZE } from "$lib/constants";
import type { PageLoad } from "./$types";

export type Artist = {
  id: string,
  name: string,
  country: string | null,
  disambiguation: string | null
}

export type ArtistSearchResp = {
  artist_count: number,
  artists: Artist[]
}

export const load: PageLoad = async ({ fetch, url }) => {
  const q = url.searchParams.get("q") ?? "";
  const page = Number(url.searchParams.get("page") ?? "0");

  if (q === "") {
    return {
      query: "",
      page: 0,
      pages: 0,
      artists: null
    };
  }

  const resp = await fetch(`${API_URL}/mb/search_artist/${encodeURIComponent(q)}?page=${page}`);
  const body: ArtistSearchResp = await resp.json();
  return {
    query: q,
    page,
    pages: Math.ceil(body.artist_count / PAGE_SIZE),
    artists: body.artists
  };
};
