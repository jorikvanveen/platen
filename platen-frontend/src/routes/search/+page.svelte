<script lang="ts">
	import { CircleAlert, LoaderCircle, Search, SearchX } from '@lucide/svelte';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { navigating } from '$app/state';
	import AddAlbumCard from '$lib/components/AddAlbumCard.svelte';
	import { Button } from '$lib/components/ui/button/index.js';
	import * as Card from '$lib/components/ui/card/index.js';
	import { Input } from '$lib/components/ui/input/index.js';
	import { Label } from '$lib/components/ui/label/index.js';
	import type { PageProps } from './$types';

	let { data }: PageProps = $props();
	let searchQuery = $derived(data.query);
	const isSearching = $derived(navigating.to?.route.id === '/search');

	function submitSearch(event: SubmitEvent) {
		event.preventDefault();
		searchQuery = searchQuery.trim();
		const searchParams = new URLSearchParams();
		if (searchQuery) searchParams.set('query', searchQuery);

		// SvelteKit otherwise reuses this loader's data when the query URL is unchanged.
		void goto(`${resolve('/search')}?${searchParams}`, {
			invalidateAll: true,
			keepFocus: true,
			noScroll: true
		});
	}
</script>

<svelte:head>
	<title>Search albums · Platen</title>
	<meta name="description" content="Search Tidal for albums to add to your Platen catalog." />
</svelte:head>

<section aria-labelledby="search-heading">
	<div class="page-header">
		<h1 id="search-heading">Search albums</h1>
	</div>

	<form
		id="album-search"
		class="search-form"
		role="search"
		method="GET"
		action={resolve('/search')}
		onsubmit={submitSearch}
	>
		<Label for="album-query">Album or artist</Label>
		<div class="search-controls">
			<Input
				id="album-query"
				name="query"
				type="search"
				class="h-11 bg-card"
				bind:value={searchQuery}
				placeholder="Album title, artist name, or both"
				required
			/>
			<Button type="submit" class="h-11 px-5" disabled={isSearching}>
				{#if isSearching}
					<LoaderCircle class="motion-safe:animate-spin" aria-hidden="true" />
					Searching...
				{:else}
					<Search aria-hidden="true" />
					Search
				{/if}
			</Button>
		</div>
	</form>

	<p class="results-summary" role="status">
		{#if isSearching}
			Searching Tidal...
		{:else if data.results && data.results.albums.length > 0}
			{data.results.albums.length}
			{data.results.albums.length === 1 ? 'result' : 'results'} for "{data.query}"
		{:else if data.results}
			<span class="sr-only">No albums found for "{data.query}".</span>
		{/if}
	</p>

	<div aria-busy={isSearching}>
		{#if data.results && data.results.albums.length > 0}
			<ul class="album-grid" aria-label="Album search results" role="list">
				{#each data.results.albums as album (album.id)}
					<li><AddAlbumCard {album} /></li>
				{/each}
			</ul>
		{:else}
			<Card.Root class="gap-0 border-dashed py-0 shadow-none">
				<Card.Content class="p-0">
					<div class="empty-state" role={data.searchError ? 'alert' : undefined}>
						{#if data.searchError}
							<CircleAlert size={30} strokeWidth={1.5} aria-hidden="true" />
							<h2>Could not search for albums</h2>
							<p>{data.searchError}</p>
							<Button type="submit" form="album-search" variant="outline" disabled={isSearching}>
								Try again
							</Button>
						{:else if !data.results}
							<Search size={30} strokeWidth={1.5} aria-hidden="true" />
							<h2>Search by album or artist</h2>
						{:else}
							<SearchX size={30} strokeWidth={1.5} aria-hidden="true" />
							<h2>No albums found</h2>
						{/if}
					</div>
				</Card.Content>
			</Card.Root>
		{/if}
	</div>
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

	.search-form {
		display: flex;
		max-width: 44rem;
		flex-direction: column;
		gap: 0.625rem;
	}

	.search-controls {
		display: flex;
		gap: 0.75rem;
	}

	.results-summary {
		min-height: 1.5rem;
		margin-top: 2.5rem;
		margin-bottom: 1rem;
		color: var(--muted-foreground);
		font-size: 0.875rem;
		line-height: 1.5;
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
		text-align: center;
		color: var(--muted-foreground);
	}

	.empty-state h2 {
		color: var(--foreground);
		font-size: 1.0625rem;
		font-weight: 600;
	}

	.empty-state p {
		max-width: 30rem;
		font-size: 0.875rem;
		line-height: 1.6;
		overflow-wrap: anywhere;
	}

	@media (max-width: 40rem) {
		.page-header {
			margin-bottom: 1.5rem;
		}

		.search-controls {
			flex-direction: column;
		}

		.results-summary {
			margin-top: 2rem;
		}

		.album-grid {
			gap: 1rem;
		}
	}
</style>
