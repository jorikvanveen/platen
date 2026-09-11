<script lang="ts">
	import { Check, LoaderCircle, Plus, RotateCcw } from '@lucide/svelte';
	import { invalidate } from '$app/navigation';
	import AlbumCard from '$lib/components/AlbumCard.svelte';
	import { Button } from '$lib/components/ui/button/index.js';
	import { API_URL } from '$lib/constants';
	import type { TidalAlbumSearchHit } from '$lib/dto/TidalAlbumSearchHit';

	let { album }: { album: TidalAlbumSearchHit } = $props();
	let catalogState = $state<'idle' | 'adding' | 'added' | 'failed'>('idle');

	async function addToCatalog() {
		if (catalogState === 'adding' || catalogState === 'added') return;
		catalogState = 'adding';

		try {
			const response = await fetch(`${API_URL}/albums/${encodeURIComponent(album.id)}`, {
				method: 'POST'
			});

			catalogState = response.ok ? 'added' : 'failed';
		} catch {
			catalogState = 'failed';
		}

		if (catalogState === 'added') {
			// A failed refresh must not turn a successful catalog addition into an error.
			await invalidate(`${API_URL}/artists`).catch(() => {});
		}
	}
</script>

<AlbumCard {album} releaseYear={album.release_date?.slice(0, 4)}>
	<div class="catalog-action">
		<p class="sr-only" role="status">
			{#if catalogState === 'adding'}
				Adding {album.title} to your catalog.
			{:else if catalogState === 'added'}
				{album.title} added to your catalog.
			{/if}
		</p>
		{#if catalogState === 'failed'}
			<p class="add-error" role="alert">Could not add this album. Try again.</p>
		{/if}
		<Button
			class="h-10 w-full"
			variant={catalogState === 'added' ? 'secondary' : 'outline'}
			disabled={catalogState === 'adding' || catalogState === 'added'}
			onclick={addToCatalog}
		>
			{#if catalogState === 'adding'}
				<LoaderCircle class="motion-safe:animate-spin" aria-hidden="true" />
				Adding...
			{:else if catalogState === 'added'}
				<Check aria-hidden="true" />
				Added
			{:else if catalogState === 'failed'}
				<RotateCcw aria-hidden="true" />
				Retry
			{:else}
				<Plus aria-hidden="true" />
				Add to catalog
			{/if}
			<span class="sr-only"> {album.title}</span>
		</Button>
	</div>
</AlbumCard>

<style>
	.add-error {
		margin-bottom: 0.75rem;
		color: var(--destructive);
		font-size: 0.8125rem;
		line-height: 1.5;
	}
</style>
