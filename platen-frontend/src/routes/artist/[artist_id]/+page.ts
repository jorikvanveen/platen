import { API_URL } from "$lib/constants";
import type { PageLoad } from "./$types";

export type Artist = {
  musicbrainz_id: string,
  name: string
}

export type Release = {
  musicbrainz_id: string,
  title: string,
  artist_id: string,
  downloaded: boolean
}

export const load: PageLoad = async ({ fetch, params }) => {
  const artist_req = fetch(`${API_URL}/artist/${params.artist_id}`);
  const releases_req = fetch(`${API_URL}/artist/${params.artist_id}/release-groups`);
  const artist_resp = await artist_req;
  const releases_resp = await releases_req;
  const artist: Artist = await artist_resp.json();
  const releases: Release[] = await releases_resp.json();
  console.log(artist)
  console.log(releases)
  return {
    artist,
    releases
  }
}
