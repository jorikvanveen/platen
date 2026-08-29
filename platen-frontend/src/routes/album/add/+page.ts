import type { TidalAlbumSearchHit } from '$lib/dto/TidalAlbumSearchHit';
import { loadSearchResults } from '$lib/searchLoader';
import type { PageLoad } from './$types';

export const load: PageLoad = async ({ fetch, url }) => {
	const { query, results } = await loadSearchResults<TidalAlbumSearchHit>(
		fetch,
		url,
		'/tidal/search/albums',
		'Album search failed'
	);

	return { query, albums: results };
};
