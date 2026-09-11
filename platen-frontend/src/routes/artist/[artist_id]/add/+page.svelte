<script lang="ts">
	import { ArrowLeft, CircleAlert, Disc3, LoaderCircle } from '@lucide/svelte';
	import { invalidate } from '$app/navigation';
	import { resolve } from '$app/paths';
	import ArtistProfileImage from '$lib/components/ArtistProfileImage.svelte';
	import ArtistReleaseCard from '$lib/components/ArtistReleaseCard.svelte';
	import { Badge } from '$lib/components/ui/badge/index.js';
	import { Button } from '$lib/components/ui/button/index.js';
	import * as Card from '$lib/components/ui/card/index.js';
	import { API_URL } from '$lib/constants';
	import type { PageProps } from './$types';

	let { data }: PageProps = $props();
	let retrying = $state(false);
	let discoveryError = $derived(data.discoveryError);

	async function retryDiscovery() {
		if (retrying) return;
		retrying = true;
		try {
			await invalidate(`${API_URL}/tidal/artists/${encodeURIComponent(data.artist.id)}`);
		} catch {
			discoveryError = 'Could not reach Tidal. Check your connection and try again.';
		} finally {
			retrying = false;
		}
	}
</script>

<svelte:head>
	<title>Add release · {data.artist.name} · Platen</title>
	<meta
		name="description"
		content={`Find releases by ${data.artist.name} to add and download.`}
	/>
</svelte:head>

<Button
	href={resolve('/artist/[artist_id]', { artist_id: data.artist.id })}
	variant="ghost"
	class="-ml-3 mb-6"
>
	<ArrowLeft aria-hidden="true" />
	Back to artist
</Button>

<section aria-labelledby="add-release-heading">
	<div class="page-header">
		<div class="profile-image" aria-hidden="true">
			<ArtistProfileImage artist={data.artist} />
		</div>
		<div class="page-title">
			<h1 id="add-release-heading">Add release</h1>
			<p class="artist-name">{data.artist.name}</p>
		</div>
		{#if data.results && data.results.albums.length > 0}
			<Badge variant="secondary">
				{data.results.albums.length}
				{data.results.albums.length === 1 ? 'release' : 'releases'}
			</Badge>
		{/if}
	</div>

	{#if data.results && data.results.albums.length > 0}
		<ul class="album-grid" aria-label="Releases to add" role="list">
			{#each data.results.albums as album (album.id)}
				<li><ArtistReleaseCard {album} /></li>
			{/each}
		</ul>
	{:else}
		<Card.Root class="gap-0 border-dashed py-0 shadow-none">
			<Card.Content class="p-0">
				<div
					class="empty-state"
					role={discoveryError ? 'alert' : undefined}
					aria-busy={retrying}
				>
					{#if discoveryError}
						<CircleAlert size={32} strokeWidth={1.5} aria-hidden="true" />
						<h2>Could not load releases</h2>
						<p>{discoveryError}</p>
						<Button variant="outline" onclick={retryDiscovery} disabled={retrying}>
							{#if retrying}
								<LoaderCircle class="motion-safe:animate-spin" aria-hidden="true" />
								Loading...
							{:else}
								Try again
							{/if}
						</Button>
					{:else}
						<Disc3 size={32} strokeWidth={1.5} aria-hidden="true" />
						<h2>No releases to add</h2>
					{/if}
				</div>
			</Card.Content>
		</Card.Root>
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

	.page-title {
		min-width: 0;
		flex: 1;
	}

	h1 {
		font-size: clamp(1.875rem, 4vw, 2.25rem);
		font-weight: 650;
		letter-spacing: -0.04em;
		line-height: 1.2;
	}

	.artist-name {
		margin-top: 0.75rem;
		font-size: 1rem;
		overflow-wrap: anywhere;
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

	.empty-state p {
		max-width: 30rem;
		font-size: 0.875rem;
		line-height: 1.5;
		overflow-wrap: anywhere;
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

		.page-title {
			flex-basis: calc(100% - 6.25rem);
		}

		.album-grid {
			gap: 1rem;
		}
	}
</style>
