<script lang="ts">
	import type { Snippet } from 'svelte';
	import { resolve } from '$app/paths';
	import ArtistAlbumCounts from '$lib/components/ArtistAlbumCounts.svelte';
	import ArtistProfileImage from '$lib/components/ArtistProfileImage.svelte';
	import type { ArtistSummary } from '$lib/dto/ArtistSummary';

	let { artist, monitoring }: { artist: ArtistSummary; monitoring: Snippet } = $props();
</script>

<div class="artist-row">
	<a class="artist-link" href={resolve('/artist/[artist_id]', { artist_id: artist.id })}>
		<div class="profile-image" aria-hidden="true">
			<ArtistProfileImage {artist} compact />
		</div>
		<span class="artist-name">{artist.name}</span>
	</a>
	<div class="monitor-control">{@render monitoring()}</div>
	<div class="album-count"><ArtistAlbumCounts {artist} /></div>
</div>

<style>
	.artist-row {
		position: relative;
		display: grid;
		grid-template-columns: minmax(0, 1fr) 6.5rem 9.5rem;
		column-gap: 1rem;
		align-items: center;
		min-height: 4rem;
		padding: 0.625rem 1.25rem;
		color: inherit;
		transition: background-color 120ms;
	}

	.artist-row:hover {
		background: var(--muted);
	}

	.artist-link {
		display: flex;
		min-width: 0;
		align-items: center;
		gap: 1rem;
		color: inherit;
		text-decoration: none;
	}

	.artist-link::after {
		position: absolute;
		inset: 0;
		content: '';
	}

	.profile-image {
		width: 2.5rem;
		height: 2.5rem;
		flex-shrink: 0;
	}

	.artist-name {
		min-width: 0;
		font-size: 0.9375rem;
		font-weight: 500;
		line-height: 1.5;
		overflow-wrap: anywhere;
	}

	.monitor-control {
		position: relative;
		z-index: 1;
		display: flex;
		justify-content: center;
	}

	.album-count {
		text-align: right;
	}

	.artist-link:focus-visible {
		outline: none;
	}

	.artist-row:has(.artist-link:focus-visible) {
		outline: 2px solid var(--ring);
		outline-offset: -3px;
	}

	@media (max-width: 40rem) {
		.artist-row {
			grid-template-columns: minmax(0, 1fr) 2.75rem;
			column-gap: 0.5rem;
			padding-inline: 1rem;
		}

		.album-count {
			display: none;
		}
	}

	@media (prefers-reduced-motion: reduce) {
		.artist-row {
			transition: none;
		}
	}
</style>
