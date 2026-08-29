import { error } from '@sveltejs/kit';
import { API_URL } from '$lib/constants';
import type { Artist } from '$lib/dto/Artist';
import type { TidalAlbum } from '$lib/dto/TidalAlbum';
import type { PageLoad } from './$types';

export const load: PageLoad = async ({ fetch, params }) => {
	const [artistResponse, albumsResponse] = await Promise.all([
		fetch(`${API_URL}/artists/${params.artist_id}`),
		fetch(`${API_URL}/tidal/artists/${params.artist_id}`)
	]);
	if (!artistResponse.ok) throw error(artistResponse.status, 'Artist not found');
	if (!albumsResponse.ok) throw error(albumsResponse.status, 'Could not load releases');

	return {
		artist: (await artistResponse.json()) as Artist,
		albums: (await albumsResponse.json()) as TidalAlbum[]
	};
};
