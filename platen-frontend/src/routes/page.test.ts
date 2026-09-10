import { render } from 'svelte/server';
import { describe, expect, it, vi } from 'vitest';
import { API_URL } from '$lib/constants';
import type { Artist } from '$lib/dto/Artist';
import { load } from './+page';
import Page from './+page.svelte';

describe('Artist homepage', () => {
	it('loads and displays every artist alphabetically, ignoring case and accents', async () => {
		const artists: Artist[] = [
			{ id: '1', name: 'Zebra', profile_image_url: null },
			{ id: '2', name: 'Édith Piaf', profile_image_url: null },
			{ id: '3', name: 'ABBA', profile_image_url: null },
			{ id: '4', name: 'earth, wind & fire', profile_image_url: null },
			{ id: '5', name: 'ABBA', profile_image_url: null }
		];
		const fetch = vi.fn<typeof globalThis.fetch>().mockResolvedValue(Response.json(artists));

		const data = await load({ fetch } as unknown as Parameters<typeof load>[0]);

		expect(fetch).toHaveBeenCalledExactlyOnceWith(`${API_URL}/artists`);
		expect(data).toEqual({ artists: [artists[2], artists[4], artists[3], artists[1], artists[0]] });

		const html = render(Page, { props: { data, params: {} } }).body;
		const displayedNames = [...html.matchAll(/<li[^>]*>(.*?)<\/li>/g)].map((match) => match[1]);
		expect(displayedNames).toEqual(['ABBA', 'ABBA', 'earth, wind &amp; fire', 'Édith Piaf', 'Zebra']);
		expect(html).not.toContain('<a ');
	});

	it('shows an empty state when there are no artists', async () => {
		const fetch = vi.fn<typeof globalThis.fetch>().mockResolvedValue(Response.json([]));

		const data = await load({ fetch } as unknown as Parameters<typeof load>[0]);
		const html = render(Page, { props: { data, params: {} } }).body;

		expect(data).toEqual({ artists: [] });
		expect(html).toContain('No artists in the catalog yet.');
		expect(html).not.toContain('<ul');
	});

	it('preserves API failures instead of showing an empty catalog', async () => {
		const fetch = vi.fn<typeof globalThis.fetch>().mockResolvedValue(new Response(null, { status: 503 }));

		await expect(load({ fetch } as unknown as Parameters<typeof load>[0])).rejects.toMatchObject({
			status: 503,
			body: { message: 'Could not load artists' }
		});
	});
});
