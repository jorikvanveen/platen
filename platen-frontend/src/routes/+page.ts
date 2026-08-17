export type Artist = {
  musicbrainz_id: string,
  name: string
}

import type { PageLoad } from './$types';
export const load: PageLoad = async ({ fetch }) => {
  const resp = await fetch("/api/artist");
  const artists: Artist[] = await resp.json()
  return {
    artists
  }
}
