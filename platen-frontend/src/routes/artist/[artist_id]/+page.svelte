<script lang="ts">
	import { ArrowDownAZ, ArrowLeft, Disc3 } from '@lucide/svelte';
	import { resolve } from '$app/paths';
	import AlbumCard from '$lib/components/AlbumCard.svelte';
	import ArtistProfileImage from '$lib/components/ArtistProfileImage.svelte';
	import { Badge } from '$lib/components/ui/badge/index.js';
	import { Button } from '$lib/components/ui/button/index.js';
	import * as Card from '$lib/components/ui/card/index.js';
	import type { PageProps } from './$types';

	let { data }: PageProps = $props();
</script>

<svelte:head>
	<title>{data.artist.name} · Platen</title>
	<meta
		name="description"
		content={`Catalog albums credited to ${data.artist.name}, sorted by title.`}
	/>
</svelte:head>

<Button href={resolve('/')} variant="ghost" class="-ml-3 mb-6">
	<ArrowLeft aria-hidden="true" />
	Artists
</Button>

<section aria-labelledby="artist-heading">
	<div class="page-header">
		<div class="profile-image" aria-hidden="true">
			<ArtistProfileImage artist={data.artist} />
		</div>
		<div class="artist-details">
			<h1 id="artist-heading">{data.artist.name}</h1>
			<Badge variant="secondary">
				{data.albums.length} {data.albums.length === 1 ? 'album' : 'albums'}
			</Badge>
		</div>

		{#if data.albums.length > 0}
			<p class="sort-order">
				<ArrowDownAZ size={16} aria-hidden="true" />
				<span>Title, A–Z</span>
			</p>
		{/if}
	</div>

	{#if data.albums.length === 0}
		<Card.Root class="gap-0 border-dashed py-0 shadow-none">
			<Card.Content class="p-0">
				<div class="empty-state">
					<Disc3 size={32} strokeWidth={1.5} aria-hidden="true" />
					<h2>No albums in the catalog yet.</h2>
					<Button href={resolve('/search')}>Search albums</Button>
				</div>
			</Card.Content>
		</Card.Root>
	{:else}
		<ul class="album-grid" aria-label="Albums, sorted alphabetically" role="list">
			{#each data.albums as album (album.id)}
				<li><AlbumCard {album} releaseYear={album.release_year} /></li>
			{/each}
		</ul>
	{/if}
</section>

<style>
	.page-header {
		display: flex;
		align-items: center;
		gap: 1.5rem;
		margin-bottom: 2.5rem;
	}

	.profile-image {
		width: 7rem;
		flex-shrink: 0;
		aspect-ratio: 1;
	}

	.artist-details {
		display: flex;
		min-width: 0;
		flex: 1;
		flex-direction: column;
		align-items: flex-start;
		gap: 0.875rem;
	}

	h1 {
		font-size: clamp(1.875rem, 4vw, 2.25rem);
		font-weight: 650;
		letter-spacing: -0.04em;
		line-height: 1.2;
		overflow-wrap: anywhere;
	}

	.sort-order {
		display: flex;
		flex-shrink: 0;
		align-items: center;
		gap: 0.5rem;
		color: var(--muted-foreground);
		font-size: 0.8125rem;
	}

	.album-grid {
		display: grid;
		grid-template-columns: repeat(auto-fill, minmax(min(100%, 14rem), 1fr));
		gap: 1.25rem;
		padding: 0;
		list-style: none;
	}

	.album-grid > li {
		min-width: 0;
	}

	.empty-state {
		display: flex;
		min-height: 20rem;
		flex-direction: column;
		align-items: center;
		justify-content: center;
		gap: 1rem;
		padding: 3rem 1.5rem;
		color: var(--muted-foreground);
		text-align: center;
	}

	.empty-state h2 {
		color: var(--foreground);
		font-size: 1.0625rem;
		font-weight: 600;
	}

	@media (max-width: 40rem) {
		.page-header {
			flex-wrap: wrap;
			gap: 1.25rem;
			margin-bottom: 2rem;
		}

		.profile-image {
			width: 5rem;
		}

		.sort-order {
			width: 100%;
		}

		.album-grid {
			gap: 1rem;
		}
	}
</style>
