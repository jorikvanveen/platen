import { API_URL } from '$lib/constants';
import type { Artist } from '$lib/dto/Artist';
import type { Album } from '$lib/dto/Album';
import type { PageLoad } from './$types';

export const load: PageLoad = async ({ fetch, params }) => {
  const artist_req = fetch(`${API_URL}/artists/${params.artist_id}`);
  const albums_req = fetch(`${API_URL}/artists/${params.artist_id}/albums`);
  const artist_resp = await artist_req;
  const albums_resp = await albums_req;
  const artist: Artist = await artist_resp.json();
  const albums: Album[] = await albums_resp.json();
  console.log(artist)
  console.log(albums)
  return {
    artist,
    albums
  }
}
