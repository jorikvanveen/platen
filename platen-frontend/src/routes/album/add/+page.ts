import { error } from '@sveltejs/kit';
import { API_URL } from '$lib/constants';
import type { TidalAlbumSearchResults } from '$lib/dto/TidalAlbumSearchResults';
import type { PageLoad } from './$types';

export const load: PageLoad = async ({ fetch, url }) => {
	const query = url.searchParams.get('q')?.trim() ?? '';
	if (query === '') return { query, albums: null, returnedCount: 0 };

	const response = await fetch(`${API_URL}/tidal/search/albums?query=${encodeURIComponent(query)}`);
	if (!response.ok) throw error(response.status, 'Album search failed');
	const results = (await response.json()) as TidalAlbumSearchResults;

	return { query, albums: results.albums, returnedCount: results.returned_count };
};
