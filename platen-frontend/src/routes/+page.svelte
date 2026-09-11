<script lang="ts">
	import { UsersRound } from '@lucide/svelte';
	import { resolve } from '$app/paths';
	import ArtistCard from '$lib/components/ArtistCard.svelte';
	import { Button } from '$lib/components/ui/button/index.js';
	import * as Card from '$lib/components/ui/card/index.js';
	import type { PageProps } from './$types';

	let { data }: PageProps = $props();
</script>

<svelte:head>
	<title>Artists · Platen</title>
	<meta name="description" content="Browse all the artists in your Platen catalog, sorted by name." />
</svelte:head>

<section aria-labelledby="artists-heading">
	<div class="page-header">
		<h1 id="artists-heading">Artists</h1>
		<p class="page-description">All the artists in your catalog.</p>
	</div>

	{#if data.artists.length === 0}
		<Card.Root class="gap-0 border-dashed py-0 shadow-none">
			<Card.Content class="p-0">
				<div class="empty-state">
					<div class="empty-icon" aria-hidden="true">
						<UsersRound size={26} strokeWidth={1.5} />
					</div>
					<h2>No artists in the catalog yet.</h2>
					<p>Artists will appear here when albums are added to your catalog.</p>
					<Button href={resolve('/search')} class="mt-6">Search albums</Button>
				</div>
			</Card.Content>
		</Card.Root>
	{:else}
		<ul class="artist-grid" aria-label="Artists, sorted alphabetically" role="list">
			{#each data.artists as artist (artist.id)}
				<li><ArtistCard {artist} /></li>
			{/each}
		</ul>
	{/if}
</section>

<style>
	.page-header {
		margin-bottom: 2rem;
	}

	h1 {
		font-size: clamp(1.875rem, 4vw, 2.25rem);
		font-weight: 650;
		letter-spacing: -0.04em;
		line-height: 1.2;
	}

	.page-description {
		margin-top: 0.625rem;
		color: var(--muted-foreground);
		font-size: 0.9375rem;
		line-height: 1.6;
	}

	.artist-grid {
		display: grid;
		grid-template-columns: repeat(auto-fill, minmax(min(100%, 12.5rem), 1fr));
		gap: 1.25rem;
		padding: 0;
		list-style: none;
	}

	.artist-grid > li {
		min-width: 0;
	}

	.empty-state {
		display: flex;
		min-height: 22rem;
		flex-direction: column;
		align-items: center;
		justify-content: center;
		padding: 3rem 1.5rem;
		text-align: center;
	}

	.empty-icon {
		display: grid;
		width: 3.5rem;
		height: 3.5rem;
		margin-bottom: 1.25rem;
		place-items: center;
		border: 1px solid var(--border);
		border-radius: 1rem;
		background: var(--muted);
		color: var(--muted-foreground);
	}

	.empty-state h2 {
		font-size: 1.0625rem;
		font-weight: 600;
	}

	.empty-state p {
		max-width: 22rem;
		margin-top: 0.5rem;
		color: var(--muted-foreground);
		font-size: 0.875rem;
		line-height: 1.6;
	}

	@media (max-width: 40rem) {
		.page-header {
			margin-bottom: 1.5rem;
		}

		.artist-grid {
			grid-template-columns: repeat(2, minmax(0, 1fr));
			gap: 0.75rem;
		}
	}

	@media (max-width: 22rem) {
		.artist-grid {
			grid-template-columns: minmax(0, 1fr);
		}
	}
</style>
