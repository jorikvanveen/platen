import { API_URL } from "$lib/constants";
import type { PageLoad } from "./$types";

export type Artist = {
  musicbrainz_id: string,
  name: string
}

export type ReleaseGroupResp = {
  release_group_count: number,
  release_groups: {
    primary_type: string,
    disambiguation: string,
    id: string,
    first_release_date: string,
    title: string
  }[]
}

export const load: PageLoad = async ({ fetch, params }) => {
  const artist_req = fetch(`${API_URL}/artist/${params.artist_id}`);
  const artist_resp = await artist_req;
  const artist: Artist = await artist_resp.json();

  const releases_req = await fetch(`${API_URL}/mb/artist/${params.artist_id}/release-groups?page=${params.page}`)
  const releases_resp: ReleaseGroupResp = await releases_req.json()
  console.log(releases_req.status)
  return {
    artist,
    page: params.page,
    pages: Math.floor(releases_resp.release_group_count / 100),
    release_groups: releases_resp.release_groups
  }
}
