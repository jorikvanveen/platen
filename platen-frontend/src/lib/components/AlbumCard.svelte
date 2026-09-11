<script lang="ts">
	import { Music2 } from '@lucide/svelte';
	import type { Snippet } from 'svelte';
	import * as Avatar from '$lib/components/ui/avatar/index.js';
	import { Badge } from '$lib/components/ui/badge/index.js';
	import * as Card from '$lib/components/ui/card/index.js';
	import type { Album } from '$lib/dto/Album';

	let {
		album,
		releaseYear,
		children
	}: {
		album: Pick<Album, 'title' | 'cover_url' | 'album_type' | 'artists' | 'explicit'>;
		releaseYear: number | string | undefined;
		children?: Snippet;
	} = $props();

	const albumTypeLabels: Record<string, string> = {
		album: 'Album',
		ep: 'EP',
		single: 'Single',
		compilation: 'Compilation'
	};
	const albumType = $derived(
		album.album_type
			? (albumTypeLabels[album.album_type.toLowerCase()] ?? album.album_type)
			: null
	);
	const artistNames = $derived(album.artists.map((artist) => artist.name).join(', '));
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
					{#if releaseYear !== undefined}<span>{releaseYear}</span>{/if}
					{#if albumType}<span>{albumType}</span>{/if}
					{#if album.explicit}<Badge variant="outline">Explicit</Badge>{/if}
				</div>
			</div>

			{@render children?.()}
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

	@media (max-width: 40rem) {
		.album-content {
			padding: 1rem;
		}
	}
</style>
