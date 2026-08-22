import { API_URL } from '$lib/constants';
import type { Artist } from '$lib/dto/Artist';
import type { TidalAlbum } from '$lib/dto/TidalAlbum';
import type { PageLoad } from './$types';

export const load: PageLoad = async ({ fetch, params }) => {
  const artist_resp = await fetch(`${API_URL}/artists/${params.artist_id}`);
  const artist: Artist = await artist_resp.json();

  const albums_resp = await fetch(`${API_URL}/tidal/artists/${params.artist_id}`);
  const albums: TidalAlbum[] = await albums_resp.json();
  console.log(albums)
  return {
    artist,
    albums
  }
}
