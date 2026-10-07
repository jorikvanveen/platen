<script lang="ts">
	import type { ArtistSummary } from '$lib/dto/ArtistSummary';

	let { artist, showLabel = false }: { artist: ArtistSummary; showLabel?: boolean } = $props();
	const description = $derived(
		`${artist.album_count} Catalog Albums, ${artist.downloaded_album_count} Downloaded Albums`
	);
</script>

<div class="album-counts" title={description}>
	{#if showLabel}<span class="count-label" aria-hidden="true">Albums/Downloaded</span>{/if}
	<span class="count-value" aria-hidden="true">
		{artist.album_count}/{artist.downloaded_album_count}
	</span>
	<span class="sr-only">{description}</span>
</div>

<style>
	.album-counts {
		min-width: 0;
	}

	.count-label {
		display: block;
		color: var(--muted-foreground);
		font-size: 0.6875rem;
		line-height: 1.5;
	}

	.count-value {
		display: block;
		font-size: 0.875rem;
		font-weight: 500;
		font-variant-numeric: tabular-nums;
	}

	.count-label + .count-value {
		margin-top: 0.125rem;
	}
</style>
