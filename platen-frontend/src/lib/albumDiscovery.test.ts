import { render } from "svelte/server";
import { describe, expect, it, vi } from "vitest";
import { API_URL } from "$lib/constants";
import type { Artist } from "$lib/dto/Artist";
import type { TidalAlbumSearchHit } from "$lib/dto/TidalAlbumSearchHit";
import SearchPage from "../routes/album/add/+page.svelte";
import ReleasesPage from "../routes/artist/[artist_id]/releases/+page.svelte";
import { load as loadSearch } from "../routes/album/add/+page";
import { load as loadReleases } from "../routes/artist/[artist_id]/releases/+page";

const artist: Artist = { id: "artist", name: "Example artist", profile_image_url: null };
const album: TidalAlbumSearchHit = {
	id: "edition-2", title: "Available edition", cover_url: null,
	album_type: "ALBUM", release_date: "2026-01-01", popularity: 0, artists: [artist],
};

function searchEvent(fetch: typeof globalThis.fetch, query = "edition") {
	return { fetch, url: new URL(`https://platen.test/album/add?q=${query}`) } as Parameters<typeof loadSearch>[0];
}

function releasesEvent(fetch: typeof globalThis.fetch) {
	return { fetch, params: { artist_id: artist.id } } as Parameters<typeof loadReleases>[0];
}

describe("Album discovery loaders", () => {
	it("does not fetch for a blank search", async () => {
		const fetch = vi.fn();
		expect(await loadSearch(searchEvent(fetch, "++"))).toEqual({ query: "", albums: null, returnedCount: 0 });
		expect(fetch).not.toHaveBeenCalled();
	});

	it.each([
		{ albums: [], returned_count: 0 },
		{ albums: [], returned_count: 2 },
		{ albums: [album], returned_count: 3 },
	])("preserves eligible search results and original count $returned_count", async (results) => {
		const fetch = vi.fn().mockResolvedValue(Response.json(results));
		expect(await loadSearch(searchEvent(fetch))).toEqual({
			query: "edition", albums: results.albums, returnedCount: results.returned_count,
		});
		expect(fetch).toHaveBeenCalledExactlyOnceWith(`${API_URL}/tidal/search/albums?query=edition`);
	});

	it.each([
		{ albums: [], returned_count: 0 },
		{ albums: [], returned_count: 2 },
		{ albums: [album], returned_count: 3 },
	])("preserves eligible artist releases and original count $returned_count", async (results) => {
		const fetch = vi.fn()
			.mockResolvedValueOnce(Response.json(artist))
			.mockResolvedValueOnce(Response.json({ artist, ...results }));
		expect(await loadReleases(releasesEvent(fetch))).toEqual({
			artist, albums: results.albums, returnedCount: results.returned_count,
		});
		expect(fetch.mock.calls).toEqual([
			[`${API_URL}/artists/artist`], [`${API_URL}/tidal/artists/artist`],
		]);
	});

	it("requests fresh search data on a later load without mutating the current view", async () => {
		const fetch = vi.fn()
			.mockResolvedValueOnce(Response.json({ albums: [album], returned_count: 1 }))
			.mockResolvedValueOnce(Response.json({ albums: [], returned_count: 1 }));
		const currentView = await loadSearch(searchEvent(fetch));
		const nextView = await loadSearch(searchEvent(fetch));
		expect(currentView?.albums).toEqual([album]);
		expect(nextView?.albums).toEqual([]);
		expect(fetch).toHaveBeenCalledTimes(2);
	});

	it("reports search and release failures instead of displaying a false empty state", async () => {
		const failedFetch = vi.fn().mockResolvedValue(new Response(null, { status: 500 }));
		await expect(loadSearch(searchEvent(failedFetch))).rejects.toMatchObject({ status: 500 });
		const releasesFetch = vi.fn()
			.mockResolvedValueOnce(Response.json(artist))
			.mockResolvedValueOnce(new Response(null, { status: 500 }));
		await expect(loadReleases(releasesEvent(releasesFetch))).rejects.toMatchObject({ status: 500 });
	});
});

describe("Album discovery rendering", () => {
	it.each([0, 2])("distinguishes empty search responses from all-catalog responses with count %i", (returnedCount) => {
		const html = render(SearchPage, {
			props: { data: { query: "edition", albums: [], returnedCount }, params: {} },
		}).body;
		expect(html).toContain(returnedCount ? "All returned matches are already in your catalog" : "No albums matched");
		expect(html).not.toContain(returnedCount ? "No albums matched" : "All returned matches");
	});

	it.each([0, 2])("distinguishes empty release responses from all-catalog responses with count %i", (returnedCount) => {
		const html = render(ReleasesPage, {
			props: { data: { artist, albums: [], returnedCount }, params: { artist_id: artist.id } },
		}).body;
		expect(html).toContain(returnedCount ? "All returned releases are already in your catalog" : "No releases found.");
		expect(html).not.toContain(returnedCount ? "No releases found." : "All returned releases");
		expect(html).not.toContain("<h2");
		expect(html).toContain("available to add to your catalog");
	});

	it("preserves search ordering and gives eligible albums Add buttons", () => {
		const html = render(SearchPage, {
			props: {
				data: { query: "edition", albums: [album, { ...album, id: "edition-3", title: "Next edition" }], returnedCount: 3 },
				params: {},
			},
		}).body;
		expect(html.indexOf("Available edition")).toBeLessThan(html.indexOf("Next edition"));
		expect(html.replace(/<!--.*?-->/g, "").match(/>\s*Add\s*<\/button>/g)).toHaveLength(2);
	});

	it("groups eligible releases without empty headings", () => {
		const html = render(ReleasesPage, {
			props: {
				data: { artist, albums: [album, { ...album, id: "single", title: "Available single", album_type: "SINGLE" }], returnedCount: 4 },
				params: { artist_id: artist.id },
			},
		}).body;
		expect(html).toMatch(/<h2[^>]*>Albums<\/h2>/);
		expect(html).toMatch(/<h2[^>]*>Singles<\/h2>/);
		expect(html).not.toMatch(/<h2[^>]*>(EPs|Unknown)<\/h2>/);
		expect(html.indexOf("Available edition")).toBeLessThan(html.indexOf("Available single"));
	});
});
