import { API_URL } from '$lib/constants';
import type { Artist } from '$lib/dto/Artist';
import type { Album } from '$lib/dto/Album';
import type { TidalAlbum } from '$lib/dto/TidalAlbum';
import type { PageLoad } from './$types';

export const load: PageLoad = async ({ fetch, params }) => {
  const artist_req = fetch(`${API_URL}/artists/${params.artist_id}`);
  const albums_req = fetch(`${API_URL}/tidal/artists/${params.artist_id}`);
  const existing_albums_req = fetch(`${API_URL}/artists/${params.artist_id}/albums`);

  const [artist_resp, albums_resp, existing_albums_resp] = await Promise.all([
    artist_req,
    albums_req,
    existing_albums_req
  ]);

  const artist: Artist = await artist_resp.json();
  const albums: TidalAlbum[] = await albums_resp.json();
  const existing_albums: Album[] = await existing_albums_resp.json();

  return {
    artist,
    albums,
    existing_album_ids: existing_albums.map((album) => album.id)
  };
};
