import { API_URL } from '$lib/constants';
import type { TidalArtist } from '$lib/dto/TidalArtist';
import type { PageLoad } from './$types';

export const load: PageLoad = async ({ fetch, url }) => {
  const q = url.searchParams.get("q") ?? "";

  if (q === "") {
    return {
      query: "",
      artists: null
    };
  }

  const resp = await fetch(`${API_URL}/tidal/search/artists?query=${encodeURIComponent(q)}`);
  const artists: TidalArtist[] = await resp.json();
  return {
    query: q,
    artists
  };
};
