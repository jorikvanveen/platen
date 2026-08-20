import { API_URL } from "$lib/constants";
import type { PageLoad } from "./$types";

export type Artist = {
  musicbrainz_id: string,
  name: string
}

export type ReleaseGroup = {
  musicbrainz_id: string,
  title: string,
  artist_id: string,
  downloaded: boolean
}

export const load: PageLoad = async ({ fetch, params }) => {
  const artist_req = fetch(`${API_URL}/artist/${params.artist_id}`);
  const release_groups_req = fetch(`${API_URL}/artist/${params.artist_id}/release-groups`);
  const artist_resp = await artist_req;
  const release_groups_resp = await release_groups_req;
  const artist: Artist = await artist_resp.json();
  const release_groups: ReleaseGroup[] = await release_groups_resp.json();
  console.log(artist)
  console.log(release_groups)
  return {
    artist,
    release_groups
  }
}