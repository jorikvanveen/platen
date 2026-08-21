import { API_URL, PAGE_SIZE } from "$lib/constants";
import type { PageLoad } from "./$types";

export type Artist = {
  musicbrainz_id: string,
  name: string
}

export type ReleaseGroup = {
  primary_type: string,
  disambiguation: string,
  id: string,
  first_release_date: string,
  title: string
}

export type ReleaseGroupResp = {
  release_group_count: number,
  release_groups: ReleaseGroup[]
}

export const load: PageLoad = async ({ fetch, params, url }) => {
  const artist_req = fetch(`${API_URL}/artist/${params.artist_id}`);
  const artist_resp = await artist_req;
  const artist: Artist = await artist_resp.json();

  const page = Number(url.searchParams.get("page") ?? "0");
  const release_groups_req = await fetch(`${API_URL}/mb/artist/${params.artist_id}/release-groups?page=${page}`)
  const release_groups_resp: ReleaseGroupResp = await release_groups_req.json()
  console.log(release_groups_req.status)
  return {
    artist,
    page,
    pages: Math.ceil(release_groups_resp.release_group_count / PAGE_SIZE),
    release_groups: release_groups_resp.release_groups
  }
}