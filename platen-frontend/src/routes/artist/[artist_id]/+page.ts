import { error } from '@sveltejs/kit';
import { API_URL } from '$lib/constants';
import type { Album } from '$lib/dto/Album';
import type { Artist } from '$lib/dto/Artist';
import type { PageLoad } from './$types';

export const load: PageLoad = async ({ fetch, params }) => {
	const [artistResponse, albumsResponse] = await Promise.all([
		fetch(`${API_URL}/artists/${params.artist_id}`),
		fetch(`${API_URL}/artists/${params.artist_id}/albums`)
	]);
	if (!artistResponse.ok) throw error(artistResponse.status, 'Artist not found');
	if (!albumsResponse.ok) throw error(albumsResponse.status, 'Could not load releases');

	return {
		artist: (await artistResponse.json()) as Artist,
		albums: (await albumsResponse.json()) as Album[]
	};
};
