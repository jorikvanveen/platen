import { render } from "svelte/server";
import { describe, expect, it, vi } from "vitest";
import ArtistPage from "../routes/artist/[artist_id]/+page.svelte";
import AlbumDeletionDialog from "$lib/components/AlbumDeletionDialog.svelte";
import type { Album } from "$lib/dto/Album";
import type { Artist } from "$lib/dto/Artist";

const artist: Artist = { id: "artist-1", name: "Example Artist", profile_image_url: null };
const album: Album = {
	id: "album-1", title: "Example Album", artists: [artist], cover_url: null,
	album_type: "ALBUM", release_year: 2026, release_month: null, release_day: null,
	relative_path: "Artist/Example Album",
};

describe("Album deletion rendering", () => {
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
