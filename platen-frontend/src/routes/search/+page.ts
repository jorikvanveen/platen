import { API_URL } from '$lib/constants';
import type { TidalAlbumSearchResults } from '$lib/dto/TidalAlbumSearchResults';
import type { PageLoad } from './$types';

export const load = (async ({ fetch, url }) => {
	const query = url.searchParams.get('query')?.trim() ?? '';

	if (!query) {
		return { query, results: null, searchError: null };
	}

	try {
		const searchParams = new URLSearchParams({ query });
		const response = await fetch(`${API_URL}/tidal/search/albums?${searchParams}`);

		if (!response.ok) {
			return {
				query,
				results: null,
				searchError: 'Tidal search is unavailable. Try again in a moment.'
			};
		}

		const results = (await response.json()) as TidalAlbumSearchResults;
		return { query, results, searchError: null };
	} catch {
		return {
			query,
			results: null,
			searchError: 'Could not reach album search. Check your connection and try again.'
		};
	}
}) satisfies PageLoad;
