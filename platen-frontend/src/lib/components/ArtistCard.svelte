<script lang="ts">
	import * as Avatar from '$lib/components/ui/avatar/index.js';
	import * as Card from '$lib/components/ui/card/index.js';
	import type { Artist } from '$lib/dto/Artist';

	let { artist }: { artist: Artist } = $props();

	const artistInitials = $derived(
		artist.name
			.trim()
			.split(/\s+/)
			.slice(0, 2)
			.map((word) => Array.from(word)[0] ?? '')
			.join('')
			.toLocaleUpperCase('en') || '?'
	);
</script>

<Card.Root class="h-full gap-0 py-0 shadow-none">
	<Card.Content class="p-0">
		<div class="artist-content">
			<div class="profile-image" aria-hidden="true">
				<Avatar.Root class="size-full">
					{#if artist.profile_image_url}
						<Avatar.Image
							src={artist.profile_image_url}
							alt=""
							class="object-cover"
							loading="lazy"
							decoding="async"
						/>
					{/if}
					<Avatar.Fallback class="text-3xl font-medium tracking-tight text-muted-foreground">
						{artistInitials}
					</Avatar.Fallback>
				</Avatar.Root>
			</div>
			<h2>{artist.name}</h2>
		</div>
	</Card.Content>
</Card.Root>

<style>
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
