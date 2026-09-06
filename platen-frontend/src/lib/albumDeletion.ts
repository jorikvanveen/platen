import { API_URL } from "$lib/constants";

import type { AlbumDeletionPreview } from "$lib/dto/AlbumDeletionPreview";
import type { AlbumDeletionRequest } from "$lib/dto/AlbumDeletionRequest";
import type { AlbumDeletionResult } from "$lib/dto/AlbumDeletionResult";

type Fetcher = typeof fetch;

async function requireSuccess(response: Response, fallback: string): Promise<void> {
	if (response.ok) return;
	let message = fallback;
	try {
		const body = await response.text();
		if (body.trim()) message = body;
	} catch {
		// Keep the fallback if the response body cannot be read.
	}
	throw new Error(message);
}

export async function getAlbumDeletionPreview(fetcher: Fetcher, albumId: string): Promise<AlbumDeletionPreview> {
	const response = await fetcher(`${API_URL}/albums/${encodeURIComponent(albumId)}/deletion-preview`);
	await requireSuccess(response, "Could not load the Album deletion preview.");
	return await response.json();
}

export async function deleteAlbum(
	fetcher: Fetcher,
	albumId: string,
	deleteFiles = false,
): Promise<AlbumDeletionResult> {

	const request: AlbumDeletionRequest = { delete_files: deleteFiles };
	const response = await fetcher(`${API_URL}/albums/${encodeURIComponent(albumId)}`, {
		method: "DELETE",
		headers: { "Content-Type": "application/json" },
		body: JSON.stringify(request),
	});
	await requireSuccess(response, "Could not delete the Album.");
	return await response.json();
}

