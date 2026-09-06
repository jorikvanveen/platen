import { render } from "svelte/server";
import { describe, expect, it, vi } from "vitest";
import ArtistPage from "../routes/artist/[artist_id]/+page.svelte";
import AlbumDeletionDialog from "$lib/components/AlbumDeletionDialog.svelte";
import type { Album } from "$lib/dto/Album";
import type { Artist } from "$lib/dto/Artist";
import { load } from "../routes/artist/[artist_id]/+page";

const artist: Artist = { id: "artist-1", name: "Example Artist", profile_image_url: null };
const album: Album = {
	id: "album-1", title: "Example Album", artists: [artist], cover_url: null,
	album_type: "ALBUM", release_year: 2026, release_month: null, release_day: null,
	relative_path: "Artist/Example Album",
		explicit: null, media_tags: null, available_quality: null,
};

describe("Album deletion rendering", () => {
	it.each([
		{ explicit: true, media_tags: ["LOSSLESS"], available_quality: "LOSSLESS", label: "Explicit" },
		{ explicit: false, media_tags: ["HIRES_LOSSLESS"], available_quality: "HIRES_LOSSLESS", label: "Not explicit" },
		{ explicit: null, media_tags: ["LOSSLESS", "HIRES_LOSSLESS"], available_quality: "HIRES_LOSSLESS", label: "Unknown" },
		{ explicit: true, media_tags: ["FUTURE", "LOSSLESS"], available_quality: "LOSSLESS", label: "Explicit" },
		{ explicit: false, media_tags: ["DOLBY_ATMOS"], available_quality: "DOLBY_ATMOS", label: "Not explicit" },
		{ explicit: true, media_tags: ["LOSSLESS", "DOLBY_ATMOS"], available_quality: "LOSSLESS + DOLBY_ATMOS", label: "Explicit" },
		{ explicit: null, media_tags: ["HIRES_LOSSLESS", "DOLBY_ATMOS", "FUTURE"], available_quality: "HIRES_LOSSLESS + DOLBY_ATMOS", label: "Unknown" },
		{ explicit: null, media_tags: ["FUTURE"], available_quality: null, label: "Unknown" },
		{ explicit: false, media_tags: [], available_quality: null, label: "Not explicit" },
		{ explicit: null, media_tags: null, available_quality: null, label: "Unknown" },
	])("shows catalog metadata in the list and deletion confirmation: %j", async ({ label, ...metadata }) => {
		const selectedAlbum = { ...album, ...metadata };
		const data = { artist, albums: [selectedAlbum] };
		const fetch = vi.fn<typeof globalThis.fetch>().mockResolvedValueOnce(Response.json(artist))
			.mockResolvedValueOnce(Response.json(data.albums));
		expect(await load({ fetch, params: { artist_id: artist.id } } as unknown as Parameters<typeof load>[0])).toEqual(data);
		for (const html of [
			render(ArtistPage, { props: { data, params: { artist_id: artist.id } } }).body,
			render(AlbumDeletionDialog, { props: { album: selectedAlbum, oncancel: vi.fn(), ondeleted: vi.fn() } }).body,
		]) {
			expect(html).toMatch(new RegExp(`<dd[^>]*>${label}</dd>`));
			const values = [...html.matchAll(/<dd[^>]*>(.*?)<\/dd>/g)].map(match => match[1]);
			expect(values).toContain(metadata.available_quality ?? "Unknown");
			expect(html).toContain("Available quality");
			expect(html).toContain("not the downloaded audio format");
			expect(html).not.toContain("Clean");
			expect(html).not.toContain("FUTURE");
			if (metadata.explicit === null) expect(html).not.toContain("Not explicit");
		}
	});
	it("adds Delete to each Album row alongside its download control, with no Artist delete", () => {
		const html = render(ArtistPage, {
			props: {
				data: { artist, albums: [album, { ...album, id: "album-2", title: "Another Album", relative_path: null }] },
				params: { artist_id: artist.id },
			},
		}).body;
		expect(html.match(/<button[^>]*aria-label="Delete [^"]*"/g)).toHaveLength(2);
		expect(html).toContain('aria-label="Delete Example Album"');
		expect(html).toContain('aria-label="Delete Another Album"');
		expect(html).not.toContain('aria-label="Delete Example Artist"');
		expect(html).toContain("Downloaded");
		expect(html).toContain("Download");
		expect(html).not.toContain("<dialog");
	});

	it("offers no deletion on an Artist with no Albums", () => {
		const html = render(ArtistPage, {
			props: { data: { artist, albums: [] }, params: { artist_id: artist.id } },
		}).body;
		expect(html).not.toMatch(/<button\b/);
		expect(html).toContain("No credited releases.");
	});

	it("names the Album and allows only Cancel before the preview loads", () => {
		const oncancel = vi.fn();
		const ondeleted = vi.fn();
		const html = render(AlbumDeletionDialog, { props: { album, oncancel, ondeleted } }).body;
		expect(html).toContain('aria-labelledby="album-deletion-title"');
		expect(html).toContain("Example Album");
		expect(html).toContain("Loading deletion preview");
		expect(html.match(/<button\b/g)).toHaveLength(1);
		expect(html).toMatch(/<button[^>]*>Cancel<\/button>/);
		expect(oncancel).not.toHaveBeenCalled();
		expect(ondeleted).not.toHaveBeenCalled();
	});
});
