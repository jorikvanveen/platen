<script lang="ts">
	import { resolve } from '$app/paths';
	import ArtistProfileImage from '$lib/components/ArtistProfileImage.svelte';
	import * as Card from '$lib/components/ui/card/index.js';
	import type { Artist } from '$lib/dto/Artist';

	let { artist }: { artist: Artist } = $props();
</script>

<a class="artist-link" href={resolve('/artist/[artist_id]', { artist_id: artist.id })}>
	<Card.Root class="h-full gap-0 py-0 shadow-none">
		<Card.Content class="p-0">
			<div class="artist-content">
				<div class="profile-image" aria-hidden="true">
					<ArtistProfileImage {artist} />
				</div>
				<h2>{artist.name}</h2>
			</div>
		</Card.Content>
	</Card.Root>
</a>

<style>
	.artist-link {
		display: block;
		height: 100%;
		border-radius: var(--radius-xl);
		color: inherit;
		text-decoration: none;
	}

	.artist-link:hover h2 {
		text-decoration: underline;
		text-underline-offset: 0.25em;
	}

	.artist-link:focus-visible {
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
		.artist-content {
			gap: 1rem;
			padding: 1.25rem 0.875rem;
		}

		h2 {
			font-size: 0.875rem;
		}
	}
</style>
