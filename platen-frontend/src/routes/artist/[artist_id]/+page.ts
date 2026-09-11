import { error } from '@sveltejs/kit';
import { API_URL } from '$lib/constants';
import type { Album } from '$lib/dto/Album';
import type { Artist } from '$lib/dto/Artist';
import type { PageLoad } from './$types';

export const load: PageLoad = async ({ fetch, params }) => {
	const artistUrl = `${API_URL}/artists/${encodeURIComponent(params.artist_id)}`;
	const [artistResponse, albumsResponse] = await Promise.all([
		fetch(artistUrl),
		fetch(`${artistUrl}/albums`)
	]).catch(() => {
		error(503, 'Could not reach the catalog. Try again in a moment.');
	});

	if (!artistResponse.ok) {
		error(
			artistResponse.status,
			artistResponse.status === 404 ? 'Artist not found' : 'Could not load artist'
		);
	}
	if (!albumsResponse.ok) {
		error(
			albumsResponse.status,
			albumsResponse.status === 404 ? 'Artist not found' : 'Could not load albums'
		);
	}

	const artist = (await artistResponse.json()) as Artist;
	const albums = (await albumsResponse.json()) as Album[];
	albums.sort((firstAlbum, secondAlbum) =>
		firstAlbum.title.localeCompare(secondAlbum.title, 'en', { sensitivity: 'base' })
	);

	return { artist, albums };
};
