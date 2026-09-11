import { error } from '@sveltejs/kit';
import { API_URL } from '$lib/constants';
import type { Album } from '$lib/dto/Album';
import type { Artist } from '$lib/dto/Artist';
import type { DownloadJob } from '$lib/dto/DownloadJob';
import type { Downloads } from '$lib/dto/Downloads';
import type { PageLoad } from './$types';

export const load: PageLoad = async ({ fetch, params }) => {
	const artistUrl = `${API_URL}/artists/${encodeURIComponent(params.artist_id)}`;
	const [artistResponse, albumsResponse, downloads] = await Promise.all([
		fetch(artistUrl),
		fetch(`${artistUrl}/albums`),
		// A download-service outage must not hide the catalog.
		fetch(`${API_URL}/downloads`)
			.then((response) => (response.ok ? (response.json() as Promise<Downloads>) : null))
			.catch(() => null)
	]).catch(() => {
		error(503, 'Could not reach the catalog. Try again in a moment.');
	});

	if (!artistResponse.ok) {
		error(
			artistResponse.status,
			artistResponse.status === 404 ? 'Artist not found' : 'Could not load artist'
		);
	}
	if (!albumsResponse.ok) {
		error(
			albumsResponse.status,
			albumsResponse.status === 404 ? 'Artist not found' : 'Could not load albums'
		);
	}

	const artist = (await artistResponse.json()) as Artist;
	const albums = (await albumsResponse.json()) as Album[];
	albums.sort((firstAlbum, secondAlbum) =>
		firstAlbum.title.localeCompare(secondAlbum.title, 'en', { sensitivity: 'base' })
	);

	const downloadJobsByAlbumId = new Map<string, DownloadJob>();
	if (downloads) {
		// Keep the latest failure visible across reloads without hiding a newer retry.
		for (const job of [...downloads.active, ...downloads.history]) {
			if (
				albums.some((album) => album.id === job.album_id) &&
				!downloadJobsByAlbumId.has(job.album_id)
			) {
				downloadJobsByAlbumId.set(job.album_id, job);
			}
		}
	}

	return { artist, albums, downloadJobs: [...downloadJobsByAlbumId.values()] };
};
