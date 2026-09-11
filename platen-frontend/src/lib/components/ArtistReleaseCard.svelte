<script lang="ts">
	import { Check, Download, LoaderCircle, RotateCcw } from '@lucide/svelte';
	import { invalidate } from '$app/navigation';
	import AlbumCard from '$lib/components/AlbumCard.svelte';
	import { Button } from '$lib/components/ui/button/index.js';
	import { API_URL } from '$lib/constants';
	import { queueAlbumDownload } from '$lib/downloads';
	import type { Album } from '$lib/dto/Album';
	import type { TidalAlbum } from '$lib/dto/TidalAlbum';

	let { album }: { album: TidalAlbum } = $props();
	let catalogAlbum = $state<Album | null>(null);
	let submitting = $state(false);
	let completion = $state<'queued' | 'downloaded' | null>(null);
	let actionError = $state('');

	async function addAndDownload() {
		if (submitting || completion) return;
		submitting = true;
		actionError = '';

		if (!catalogAlbum) {
			try {
				const response = await fetch(`${API_URL}/albums/${encodeURIComponent(album.id)}`, {
					method: 'POST'
				});
				if (!response.ok) throw new Error('Catalog addition failed');
				catalogAlbum = (await response.json()) as Album;
			} catch {
				actionError = 'Could not add this release. Try again.';
				submitting = false;
				return;
			}
		}

		if (catalogAlbum.relative_path !== null) {
			completion = 'downloaded';
		} else {
			const result = await queueAlbumDownload(album.id);
			if (result.outcome === 'failed') {
				if (result.status === 404) {
					catalogAlbum = null;
					actionError = 'This release was removed from the catalog. Try adding it again.';
				} else {
					actionError = `Added to catalog. ${result.message}`;
				}
			} else {
				completion = result.outcome === 'downloaded' ? 'downloaded' : 'queued';
			}
		}
		submitting = false;

		// Keep catalog pages current without reloading discovery and hiding download retries.
		await Promise.all([
			invalidate(`${API_URL}/artists`),
			...(catalogAlbum?.artists ?? []).map((artist) =>
				invalidate(`${API_URL}/artists/${encodeURIComponent(artist.id)}/albums`)
			)
		]).catch(() => {});
	}
</script>

<AlbumCard {album} releaseYear={album.release_date?.slice(0, 4)}>
	<div class="release-action">
		<p class="sr-only" role="status">
			{#if submitting}
				{catalogAlbum ? 'Queuing a download for' : 'Adding'} {album.title}.
			{:else if completion === 'downloaded'}
				{album.title} is already downloaded.
			{:else if completion === 'queued'}
				{album.title} added to the catalog. Download queued.
			{/if}
		</p>
		{#if actionError}
			<p class="action-error" role="alert">{actionError}</p>
		{/if}
		<Button
			class="h-10 w-full"
			variant={completion ? 'secondary' : 'outline'}
			disabled={submitting || completion !== null}
			onclick={addAndDownload}
		>
			{#if submitting}
				<LoaderCircle class="motion-safe:animate-spin" aria-hidden="true" />
				{catalogAlbum ? 'Queuing...' : 'Adding...'}
			{:else if completion}
				<Check aria-hidden="true" />
				Added
			{:else if actionError}
				<RotateCcw aria-hidden="true" />
				{catalogAlbum ? 'Retry download' : 'Retry'}
			{:else}
				<Download aria-hidden="true" />
				Add & download
			{/if}
			<span class="sr-only"> {album.title}</span>
		</Button>
	</div>
</AlbumCard>

<style>
	.action-error {
		margin-bottom: 0.75rem;
		color: var(--destructive);
		font-size: 0.8125rem;
		line-height: 1.5;
		overflow-wrap: anywhere;
	}
</style>
