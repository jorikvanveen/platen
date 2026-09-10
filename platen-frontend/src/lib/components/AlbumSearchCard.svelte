<script lang="ts">
	import { Check, LoaderCircle, Music2, Plus, RotateCcw } from '@lucide/svelte';
	import { invalidate } from '$app/navigation';
	import * as Avatar from '$lib/components/ui/avatar/index.js';
	import { Badge } from '$lib/components/ui/badge/index.js';
	import { Button } from '$lib/components/ui/button/index.js';
	import * as Card from '$lib/components/ui/card/index.js';
	import { API_URL } from '$lib/constants';
	import type { TidalAlbumSearchHit } from '$lib/dto/TidalAlbumSearchHit';

	let { album }: { album: TidalAlbumSearchHit } = $props();
	let catalogState = $state<'idle' | 'adding' | 'added' | 'failed'>('idle');

	const albumTypeLabels: Record<string, string> = {
		album: 'Album',
		ep: 'EP',
		single: 'Single',
		compilation: 'Compilation'
	};
	const albumType = $derived(albumTypeLabels[album.album_type.toLowerCase()] ?? album.album_type);
	const releaseYear = $derived(album.release_date?.slice(0, 4));
	const artistNames = $derived(album.artists.map((artist) => artist.name).join(', '));

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

<Card.Root class="h-full gap-0 py-0 shadow-none">
	<div class="album-cover" aria-hidden="true">
		<Avatar.Root class="size-full rounded-none after:hidden">
			{#if album.cover_url}
				<Avatar.Image
					src={album.cover_url}
					alt=""
					class="rounded-none object-cover"
					loading="lazy"
					decoding="async"
				/>
			{/if}
			<Avatar.Fallback class="rounded-none text-foreground">
				<div class="cover-placeholder">
					<Music2 size={36} strokeWidth={1.25} />
					<span>No cover</span>
				</div>
			</Avatar.Fallback>
		</Avatar.Root>
	</div>

	<Card.Content class="flex flex-1 p-0">
		<div class="album-content">
			<div class="album-details">
				<h2>{album.title}</h2>
				<p class="artist-names">{artistNames || 'Unknown artist'}</p>
				<div class="album-metadata">
					{#if releaseYear}<span>{releaseYear}</span>{/if}
					<span>{albumType}</span>
					{#if album.explicit}<Badge variant="outline">Explicit</Badge>{/if}
				</div>
			</div>

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
		</div>
	</Card.Content>
</Card.Root>

<style>
	.album-cover {
		aspect-ratio: 1;
		width: 100%;
	}

	.cover-placeholder {
		display: flex;
		flex-direction: column;
		align-items: center;
		gap: 0.75rem;
		font-size: 0.8125rem;
	}

	.album-content {
		display: flex;
		width: 100%;
		min-width: 0;
		flex-direction: column;
		gap: 1.25rem;
		padding: 1.25rem;
	}

	.album-details {
		flex: 1;
	}

	h2 {
		font-size: 1rem;
		font-weight: 600;
		line-height: 1.4;
		overflow-wrap: anywhere;
	}

	.artist-names {
		margin-top: 0.375rem;
		color: var(--muted-foreground);
		font-size: 0.875rem;
		line-height: 1.5;
		overflow-wrap: anywhere;
	}

	.album-metadata {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 0.375rem 0.75rem;
		margin-top: 0.75rem;
		color: var(--muted-foreground);
		font-size: 0.75rem;
	}

	.add-error {
		margin-bottom: 0.75rem;
		color: var(--destructive);
		font-size: 0.8125rem;
		line-height: 1.5;
	}

	@media (max-width: 40rem) {
		.album-content {
			padding: 1rem;
		}
	}
</style>
