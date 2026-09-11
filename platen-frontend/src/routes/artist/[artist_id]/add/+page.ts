import { error } from '@sveltejs/kit';
import { API_URL } from '$lib/constants';
import type { Artist } from '$lib/dto/Artist';
import type { TidalArtistAlbums } from '$lib/dto/TidalArtistAlbums';
import type { PageLoad } from './$types';

export const load: PageLoad = async ({ fetch, params }) => {
	const artistId = encodeURIComponent(params.artist_id);
	const [artistResponse, results] = await Promise.all([
		fetch(`${API_URL}/artists/${artistId}`).catch(() => null),
		fetch(`${API_URL}/tidal/artists/${artistId}`)
			.then((response) => (response.ok ? (response.json() as Promise<TidalArtistAlbums>) : null))
			.catch(() => null)
	]);

	if (!artistResponse) {
		error(503, 'Could not reach the catalog. Try again in a moment.');
	}
	if (!artistResponse.ok) {
		error(
			artistResponse.status,
			artistResponse.status === 404 ? 'Artist not found' : 'Could not load artist'
		);
	}

	const artist = (await artistResponse.json()) as Artist;
	return {
		artist,
		results,
		discoveryError: results ? null : 'Tidal is unavailable. Try again in a moment.'
	};
};
