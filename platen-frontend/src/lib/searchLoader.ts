import { error } from '@sveltejs/kit';
import { API_URL } from '$lib/constants';

type SearchFetch = (input: RequestInfo | URL, init?: RequestInit) => Promise<Response>;

export async function loadSearchResults<T>(
	fetch: SearchFetch,
	url: URL,
	endpoint: string,
	failureMessage: string
): Promise<{ query: string; results: T[] | null }> {
	const query = url.searchParams.get('q')?.trim() ?? '';
	if (query === '') return { query, results: null };

	const response = await fetch(`${API_URL}${endpoint}?query=${encodeURIComponent(query)}`);
	if (!response.ok) throw error(response.status, failureMessage);

	return { query, results: (await response.json()) as T[] };
}
