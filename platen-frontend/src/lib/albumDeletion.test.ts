import { describe, expect, it, vi } from "vitest";
import type { AlbumDeletionPreview } from "$lib/dto/AlbumDeletionPreview";
import { deleteAlbum, getAlbumDeletionPreview } from "./albumDeletion";

const preview: AlbumDeletionPreview = {
	absolute_path: "/music/Artist/Example Album",

};

function response(body: unknown, status = 200) {
	return new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } });
}

describe("Album deletion requests", () => {
	it("loads only the directory without mutating the catalog", async () => {
		const fetcher = vi.fn().mockResolvedValue(response(preview));
		await expect(getAlbumDeletionPreview(fetcher, "album-1")).resolves.toEqual(preview);
		expect(fetcher).toHaveBeenCalledExactlyOnceWith("/api/albums/album-1/deletion-preview");
	});

	it("keeps files by default and sends an explicit JSON request", async () => {
		const result = { removed_artist_ids: ["artist-1"] };
		const fetcher = vi.fn().mockResolvedValue(response(result));
		await expect(deleteAlbum(fetcher, "album-1")).resolves.toEqual(result);
		expect(fetcher).toHaveBeenCalledExactlyOnceWith("/api/albums/album-1", {
			method: "DELETE", headers: { "Content-Type": "application/json" },
			body: JSON.stringify({ delete_files: false }),
		});
	});

	it("deletes disk files only on explicit opt-in", async () => {
		const fetcher = vi.fn().mockResolvedValue(response({ removed_artist_ids: [] }));
		await deleteAlbum(fetcher, "album-1", true);
		expect(JSON.parse(fetcher.mock.calls[0][1].body)).toEqual({ delete_files: true });
	});

	it("surfaces the server's rejection of disk deletion without a location", async () => {
		const message = "Cannot delete files without a recorded album location.";
		const fetcher = vi.fn().mockResolvedValue(new Response(message, { status: 422 }));
		await expect(deleteAlbum(fetcher, "album-1", true)).rejects.toThrow(message);
	});

	it.each([400, 404, 409, 500])("surfaces preview errors at status %s", async (status) => {
		const fetcher = vi.fn().mockResolvedValue(new Response("Album is unavailable", { status }));
		await expect(getAlbumDeletionPreview(fetcher, "album-1")).rejects.toThrow("Album is unavailable");
	});

	it.each([false, true])("allows retry after partial disk failure with delete_files=%s", async (deleteFiles) => {
		const message = "Disk deletion failed after removing some files. Catalog was not changed.";
		const fetcher = vi.fn()
			.mockResolvedValueOnce(new Response(message, { status: 500 }))
			.mockResolvedValueOnce(response({ removed_artist_ids: [] }));
		await expect(deleteAlbum(fetcher, "album-1", true)).rejects.toThrow(message);
		await expect(deleteAlbum(fetcher, "album-1", deleteFiles)).resolves.toEqual({ removed_artist_ids: [] });
		expect(JSON.parse(fetcher.mock.calls[1][1].body)).toEqual({ delete_files: deleteFiles });
	});

	it("reports plain-text errors without claiming success", async () => {
		const fetcher = vi.fn().mockResolvedValue(new Response("Bad gateway", { status: 502 }));
		await expect(deleteAlbum(fetcher, "album-1")).rejects.toThrow("Bad gateway");
	});

	it.each(["", "   "])("uses fallback messages for an empty error body %j", async (body) => {
		const fetcher = vi.fn().mockImplementation(() => Promise.resolve(new Response(body, { status: 500 })));
		await expect(deleteAlbum(fetcher, "album-1")).rejects.toThrow("Could not delete the Album.");
		await expect(getAlbumDeletionPreview(fetcher, "album-1")).rejects.toThrow("Could not load the Album deletion preview.");
	});

	it("uses fallback messages when the error body cannot be read", async () => {
		const errorResponse = new Response("Unavailable", { status: 500 });
		vi.spyOn(errorResponse, "text").mockRejectedValue(new Error("Body interrupted"));
		const fetcher = vi.fn().mockResolvedValue(errorResponse);
		await expect(deleteAlbum(fetcher, "album-1")).rejects.toThrow("Could not delete the Album.");
		await expect(getAlbumDeletionPreview(fetcher, "album-1")).rejects.toThrow("Could not load the Album deletion preview.");
	});

	it("propagates network errors", async () => {
		const fetcher = vi.fn().mockRejectedValue(new Error("Network unavailable"));
		await expect(deleteAlbum(fetcher, "album-1")).rejects.toThrow("Network unavailable");
	});
});

