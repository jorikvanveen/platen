import { error } from '@sveltejs/kit';
import { API_URL } from '$lib/constants';
import type { Artist } from '$lib/dto/Artist';
import type { PageLoad } from './$types';

export const load: PageLoad = async ({ fetch }) => {
	const response = await fetch(`${API_URL}/artists`);
	if (!response.ok) throw error(response.status, 'Could not load artists');

	return { artists: (await response.json()) as Artist[] };
};
