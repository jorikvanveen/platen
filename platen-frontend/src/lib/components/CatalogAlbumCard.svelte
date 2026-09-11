<script lang="ts">
	import { Check, Clock3, Download, LoaderCircle, RotateCcw } from '@lucide/svelte';
	import AlbumCard from '$lib/components/AlbumCard.svelte';
	import DeleteAlbumDialog from '$lib/components/DeleteAlbumDialog.svelte';
	import { Button } from '$lib/components/ui/button/index.js';
	import { queueAlbumDownload } from '$lib/downloads';
	import type { Album } from '$lib/dto/Album';
	import type { AlbumDeletionResult } from '$lib/dto/AlbumDeletionResult';
	import type { DownloadJob } from '$lib/dto/DownloadJob';

	let {
		album,
		downloadJob,
		onqueued,
		onrefresh,
		ondelete
	}: {
		album: Album;
		downloadJob?: DownloadJob;
		onqueued: (job: DownloadJob) => void;
		onrefresh: () => Promise<void>;
		ondelete: (result: AlbumDeletionResult) => void;
	} = $props();

	let submitting = $state(false);
	let requestError = $state('');
	const downloaded = $derived(album.relative_path !== null);
	const downloading = $derived(downloadJob?.status === 'running');
	const queued = $derived(downloadJob?.status === 'queued');
	const downloadError = $derived(
		requestError ||
			(downloadJob?.status === 'failed'
				? downloadJob.failure_reason || 'Download failed. Try again.'
				: '')
	);

	async function downloadAlbum() {
		if (submitting || downloaded || queued || downloading) return;
		submitting = true;
		requestError = '';

		try {
			const result = await queueAlbumDownload(album.id);
			if (result.outcome === 'downloaded') {
				requestError = 'This album is already downloaded.';
				await onrefresh();
			} else if (result.outcome === 'failed') {
				requestError = result.message;
			} else {
				onqueued(result.job);
			}
		} finally {
			submitting = false;
		}
	}
</script>

<AlbumCard {album} releaseYear={album.release_year}>
	<div class="catalog-actions">
		<p class="sr-only" role="status">
			{#if downloaded}
				{album.title} is downloaded.
			{:else if downloading}
				Downloading {album.title}.
			{:else if queued}
				{album.title} is queued for download.
			{/if}
		</p>
		{#if downloadError && !downloaded}
			<p class="download-error" role="alert">{downloadError}</p>
		{/if}
		<div class="action-buttons">
			<Button
				class="h-10 min-w-0 flex-1"
				variant={downloaded ? 'secondary' : 'outline'}
				disabled={submitting || downloaded || queued || downloading}
				onclick={downloadAlbum}
			>
				{#if downloaded}
					<Check aria-hidden="true" />Downloaded
				{:else if submitting || downloading}
					<LoaderCircle class="motion-safe:animate-spin" aria-hidden="true" />
					{submitting ? 'Queuing...' : 'Downloading...'}
				{:else if queued}
					<Clock3 aria-hidden="true" />Queued
				{:else if downloadError}
					<RotateCcw aria-hidden="true" />Retry download
				{:else}
					<Download aria-hidden="true" />Download
				{/if}
				<span class="sr-only"> {album.title}</span>
			</Button>
			<DeleteAlbumDialog
				{album}
				disabled={submitting}
				downloadActive={queued || downloading}
				{ondelete}
			/>
		</div>
	</div>
</AlbumCard>

<style>
	.action-buttons {
		display: flex;
		gap: 0.5rem;
	}

	.download-error {
		margin-bottom: 0.75rem;
		color: var(--destructive);
		font-size: 0.8125rem;
		line-height: 1.5;
		overflow-wrap: anywhere;
	}
</style>
