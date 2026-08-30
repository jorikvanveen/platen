import { error } from '@sveltejs/kit';
import { API_URL } from '$lib/constants';
import type { Artist } from '$lib/dto/Artist';
import type { TidalArtistAlbums } from '$lib/dto/TidalArtistAlbums';
import type { PageLoad } from './$types';

export const load: PageLoad = async ({ fetch, params }) => {
	const [artistResponse, albumsResponse] = await Promise.all([
		fetch(`${API_URL}/artists/${params.artist_id}`),
		fetch(`${API_URL}/tidal/artists/${params.artist_id}`)
	]);
	if (!artistResponse.ok) throw error(artistResponse.status, 'Artist not found');
	if (!albumsResponse.ok) throw error(albumsResponse.status, 'Could not load releases');

	const tidalArtist = (await albumsResponse.json()) as TidalArtistAlbums;
	return {
		artist: (await artistResponse.json()) as Artist,
		albums: tidalArtist.albums
	};
};
