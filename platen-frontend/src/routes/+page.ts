import { API_URL } from '$lib/constants';
import type { Artist } from '$lib/dto/Artist';
import type { PageLoad } from './$types';

export const load: PageLoad = async ({ fetch }) => {
  console.log(`${API_URL}/artist`);
  const resp = await fetch(`${API_URL}/artist`);
  if (!resp.ok) { 
    console.error(resp.status, await resp.text())
    throw new Error("Failed to fetch artists");
  }
  const artists: Artist[] = await resp.json()
  return {
    artists
  }
}
