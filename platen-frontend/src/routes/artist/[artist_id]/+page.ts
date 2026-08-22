import { API_URL } from "$lib/constants";
import type { Artist } from "$lib/dto/Artist";
import type { ReleaseGroup } from "$lib/dto/ReleaseGroup";
import type { PageLoad } from "./$types";

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
