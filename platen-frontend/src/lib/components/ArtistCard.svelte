<script lang="ts">
	import type { Snippet } from 'svelte';
	import { resolve } from '$app/paths';
	import ArtistAlbumCounts from '$lib/components/ArtistAlbumCounts.svelte';
	import ArtistProfileImage from '$lib/components/ArtistProfileImage.svelte';
	import * as Card from '$lib/components/ui/card/index.js';
	import type { ArtistSummary } from '$lib/dto/ArtistSummary';

	let { artist, monitoring }: { artist: ArtistSummary; monitoring: Snippet } = $props();
</script>

<div class="artist-card">
	<Card.Root class="h-full gap-0 py-0 shadow-none">
		<Card.Content class="flex h-full flex-col p-0">
			<a class="artist-link" href={resolve('/artist/[artist_id]', { artist_id: artist.id })}>
				<div class="artist-content">
					<div class="profile-image" aria-hidden="true">
						<ArtistProfileImage {artist} />
					</div>
					<h2>{artist.name}</h2>
				</div>
			</a>
			<div class="card-metadata">
				<ArtistAlbumCounts {artist} showLabel />
				<div class="monitor-control">{@render monitoring()}</div>
			</div>
		</Card.Content>
	</Card.Root>
</div>

<style>
	.artist-card {
		position: relative;
		height: 100%;
		border-radius: var(--radius-xl);
	}

	.artist-link {
		display: block;
		flex: 1;
		border-radius: var(--radius-xl);
		color: inherit;
		text-decoration: none;
	}

	.artist-link::after {
		position: absolute;
		inset: 0;
		border-radius: inherit;
		content: '';
	}

	.card-metadata {
		display: grid;
		grid-template-columns: minmax(0, 1fr) 2.75rem;
		align-items: center;
		gap: 0.375rem;
		margin-top: auto;
		padding: 0.625rem 1rem;
		border-top: 1px solid var(--border);
	}

	.monitor-control {
		position: relative;
		z-index: 1;
	}

	.artist-link:hover h2 {
		text-decoration: underline;
		text-underline-offset: 0.25em;
	}

	.artist-link:focus-visible {
		outline: none;
	}

	.artist-card:has(.artist-link:focus-visible) {
		outline: 2px solid var(--ring);
		outline-offset: 4px;
	}

	.artist-content {
		display: flex;
		flex-direction: column;
		align-items: center;
		gap: 1.25rem;
		padding: 1.5rem 1.25rem;
	}

	.profile-image {
		width: 100%;
		max-width: 10rem;
		aspect-ratio: 1;
	}

	h2 {
		max-width: 100%;
		font-size: 0.9375rem;
		font-weight: 600;
		line-height: 1.5;
		overflow-wrap: anywhere;
		text-align: center;
	}

	@media (max-width: 40rem) {
		.card-metadata {
			display: none;
		}

		.artist-content {
			gap: 1rem;
			padding: 1.25rem 0.875rem;
		}

		h2 {
			font-size: 0.875rem;
		}
	}
</style>
