<script lang="ts">
	import { ArrowLeft, Disc3, Plus } from '@lucide/svelte';
	import { tick } from 'svelte';
	import { goto, invalidate, invalidateAll } from '$app/navigation';
	import { resolve } from '$app/paths';
	import ArtistProfileImage from '$lib/components/ArtistProfileImage.svelte';
	import CatalogAlbumCard from '$lib/components/CatalogAlbumCard.svelte';
	import { Badge } from '$lib/components/ui/badge/index.js';
	import { Button } from '$lib/components/ui/button/index.js';
	import * as Card from '$lib/components/ui/card/index.js';
	import { API_URL } from '$lib/constants';
	import type { AlbumDeletionResult } from '$lib/dto/AlbumDeletionResult';
	import type { DownloadJob } from '$lib/dto/DownloadJob';
	import type { Downloads } from '$lib/dto/Downloads';
	import type { PageProps } from './$types';

	let { data }: PageProps = $props();
	let albums = $derived(data.albums);
	let downloadJobs = $derived(data.downloadJobs);
	let artistHeading: HTMLHeadingElement;
	let refreshError = $state('');
	let downloadStatusError = $state('');
	const albumsUrl = $derived(
		`${API_URL}/artists/${encodeURIComponent(data.artist.id)}/albums`
	);

	function trackDownload(job: DownloadJob) {
		downloadJobs = [...downloadJobs.filter((existing) => existing.album_id !== job.album_id), job];
	}

	async function refreshAlbums() {
		try {
			await invalidate(albumsUrl);
			refreshError = '';
		} catch {
			refreshError = 'Could not refresh the catalog. Reload the page to see the latest changes.';
		}
	}

	$effect(() => {
		const trackedJobs = downloadJobs;
		const pendingJobs = trackedJobs.filter(
			(job) => job.status === 'queued' || job.status === 'running'
		);
		if (pendingJobs.length === 0) {
			downloadStatusError = '';
			return;
		}
		const controller = new AbortController();
		let timer: ReturnType<typeof setTimeout>;

		async function pollDownloads() {
			try {
				const response = await fetch(`${API_URL}/downloads`, { signal: controller.signal });
				if (!response.ok) throw new Error('Download status failed');
				const downloads = (await response.json()) as Downloads;
				if (controller.signal.aborted) return;
				const latestJobs = [...downloads.active, ...downloads.history];
				const updatedJobs = trackedJobs.map((job) =>
					latestJobs.find((latest) => latest.id === job.id)
				);
				downloadJobs = updatedJobs.filter((job): job is DownloadJob => job !== undefined);
				downloadStatusError = '';

				if (
					pendingJobs.some((job) => {
						const latest = latestJobs.find((latest) => latest.id === job.id);
						return !latest || latest.status === 'succeeded';
					})
				) {
					await refreshAlbums();
				}
			} catch {
				if (!controller.signal.aborted) {
					downloadStatusError = 'Could not refresh download status. Retrying...';
				}
			} finally {
				if (!controller.signal.aborted) timer = setTimeout(pollDownloads, 2000);
			}
		}

		timer = setTimeout(pollDownloads, 2000);
		return () => {
			controller.abort();
			clearTimeout(timer);
		};
	});

	async function removeAlbum(albumId: string, result: AlbumDeletionResult) {
		albums = albums.filter((album) => album.id !== albumId);
		downloadJobs = downloadJobs.filter((job) => job.album_id !== albumId);
		await tick();
		artistHeading?.focus();

		try {
			if (result.removed_artist_ids.includes(data.artist.id)) {
				await goto(resolve('/'), { invalidateAll: true });
			} else {
				await invalidateAll();
			}
			refreshError = '';
		} catch {
			refreshError = 'Album deleted. Reload the page to refresh the catalog.';
		}
	}
</script>

<svelte:head>
	<title>{data.artist.name} · Platen</title>
	<meta
		name="description"
		content={`Catalog albums credited to ${data.artist.name}, sorted by title.`}
	/>
</svelte:head>

<Button href={resolve('/')} variant="ghost" class="-ml-3 mb-6">
	<ArrowLeft aria-hidden="true" />
	Artists
</Button>

<section aria-labelledby="artist-heading">
	<div class="page-header">
		<div class="profile-image" aria-hidden="true">
			<ArtistProfileImage artist={data.artist} />
		</div>
		<div class="artist-details">
			<h1 id="artist-heading" bind:this={artistHeading} tabindex="-1">{data.artist.name}</h1>
			<Badge variant="secondary">
				{albums.length} {albums.length === 1 ? 'album' : 'albums'}
			</Badge>
		</div>

		<div class="page-actions">
			<Button
				href={resolve('/artist/[artist_id]/add', { artist_id: data.artist.id })}
				class="h-10 px-4"
			>
				<Plus aria-hidden="true" />
				Add release
			</Button>
		</div>
	</div>

	{#if refreshError}<p class="page-error" role="alert">{refreshError}</p>{/if}
	{#if downloadStatusError}<p class="page-error" role="status">{downloadStatusError}</p>{/if}

	{#if albums.length === 0}
		<Card.Root class="gap-0 border-dashed py-0 shadow-none">
			<Card.Content class="p-0">
				<div class="empty-state">
					<Disc3 size={32} strokeWidth={1.5} aria-hidden="true" />
					<h2>No albums in the catalog yet.</h2>
				</div>
			</Card.Content>
		</Card.Root>
	{:else}
		<ul class="album-grid" aria-label="Albums, sorted alphabetically" role="list">
			{#each albums as album (album.id)}
				<li>
					<CatalogAlbumCard
						{album}
						downloadJob={downloadJobs.find((job) => job.album_id === album.id)}
						onqueued={trackDownload}
						onrefresh={refreshAlbums}
						ondelete={(result) => removeAlbum(album.id, result)}
					/>
				</li>
			{/each}
		</ul>
	{/if}
</section>

<style>
	.page-header {
		display: flex;
		align-items: center;
		gap: 1.5rem;
		margin-bottom: 2.5rem;
	}

	.profile-image {
		width: 7rem;
		flex-shrink: 0;
		aspect-ratio: 1;
	}

	.artist-details {
		display: flex;
		min-width: 0;
		flex: 1;
		flex-direction: column;
		align-items: flex-start;
		gap: 0.875rem;
	}

	h1 {
		font-size: clamp(1.875rem, 4vw, 2.25rem);
		font-weight: 650;
		letter-spacing: -0.04em;
		line-height: 1.2;
		overflow-wrap: anywhere;
	}

	.page-actions {
		flex-shrink: 0;
	}

	.album-grid {
		display: grid;
		grid-template-columns: repeat(auto-fill, minmax(min(100%, 14rem), 1fr));
		gap: 1.25rem;
		padding: 0;
		list-style: none;
	}

	.album-grid > li {
		min-width: 0;
	}

	.page-error {
		margin-bottom: 1.25rem;
		color: var(--destructive);
		font-size: 0.875rem;
		line-height: 1.5;
	}

	.empty-state {
		display: flex;
		min-height: 20rem;
		flex-direction: column;
		align-items: center;
		justify-content: center;
		gap: 1rem;
		padding: 3rem 1.5rem;
		color: var(--muted-foreground);
		text-align: center;
	}

	.empty-state h2 {
		color: var(--foreground);
		font-size: 1.0625rem;
		font-weight: 600;
	}

	@media (max-width: 40rem) {
		.page-header {
			flex-wrap: wrap;
			gap: 1.25rem;
			margin-bottom: 2rem;
		}

		.profile-image {
			width: 5rem;
		}

		.page-actions {
			width: 100%;
		}

		.album-grid {
			gap: 1rem;
		}
	}
</style>
