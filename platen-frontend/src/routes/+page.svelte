<script lang="ts">
	import { UsersRound } from '@lucide/svelte';
	import { onMount } from 'svelte';
	import { browser } from '$app/environment';
	import { resolve } from '$app/paths';
	import { ArtistMonitoringState } from '$lib/artist-monitoring.svelte';
	import ArtistCard from '$lib/components/ArtistCard.svelte';
	import ArtistMonitoringControl from '$lib/components/ArtistMonitoringControl.svelte';
	import ArtistRow from '$lib/components/ArtistRow.svelte';
	import CatalogList from '$lib/components/CatalogList.svelte';
	import ViewSwitcher, { type CatalogView } from '$lib/components/ViewSwitcher.svelte';
	import type { ArtistSummary } from '$lib/dto/ArtistSummary';
	import { Button } from '$lib/components/ui/button/index.js';
	import * as Card from '$lib/components/ui/card/index.js';
	import type { PageProps } from './$types';

	let { data }: PageProps = $props();

	const viewStorageKey = 'platen.homepage.artist-view';
	let view = $state<CatalogView>('cards');
	let artists = $derived(data.artists);
	// Page-owned state keeps requests alive when switching views replaces the controls.
	let monitoringByArtistId = $derived(
		Object.fromEntries(
			artists.map((artist) => [artist.id, new ArtistMonitoringState(artist.monitored)])
		)
	);

	onMount(() => {
		try {
			const savedView = localStorage.getItem(viewStorageKey);
			if (savedView === 'cards' || savedView === 'list') view = savedView;
		} catch {
			// A blocked storage API must not prevent using either view.
		}
	});

	function changeView(nextView: CatalogView) {
		view = nextView;
		if (!browser) return;
		try {
			localStorage.setItem(viewStorageKey, nextView);
		} catch {
			// Switching still works when the browser refuses to save the preference.
		}
	}
</script>

<svelte:head>
	<title>Artists · Platen</title>
	<meta name="description" content="Browse all the artists in your Platen catalog, sorted by name." />
</svelte:head>

<section aria-labelledby="artists-heading">
	<div class="page-header">
		<div>
			<h1 id="artists-heading">Artists</h1>
			<p class="page-description">All the artists in your catalog.</p>
		</div>
		{#if artists.length > 0}
			<ViewSwitcher value={view} label="Artist display view" onchange={changeView} />
		{/if}
	</div>

	{#each artists as artist (artist.id)}
		{#if monitoringByArtistId[artist.id]?.error}
			<p class="monitoring-error" id={'monitoring-error-' + artist.id} role="alert">
				{monitoringByArtistId[artist.id].error}
			</p>
		{/if}
	{/each}

	{#if artists.length === 0}
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
	{:else if view === 'cards'}
		<ul class="artist-grid" aria-label="Artists, sorted alphabetically" role="list">
			{#each artists as artist (artist.id)}
				<li>
					<ArtistCard {artist}>
						{#snippet monitoring()}{@render monitoringControl(artist)}{/snippet}
					</ArtistCard>
				</li>
			{/each}
		</ul>
	{:else}
		<CatalogList items={artists} key={(artist) => artist.id} label="Artists, sorted alphabetically">
			{#snippet header()}
				<div class="list-header" aria-hidden="true">
					<span>Artist</span>
					<span class="monitor-heading">Monitoring</span>
					<span class="count-heading">Albums/Downloaded</span>
				</div>
			{/snippet}
			{#snippet row(artist: ArtistSummary)}
				<ArtistRow {artist}>
					{#snippet monitoring()}{@render monitoringControl(artist)}{/snippet}
				</ArtistRow>
			{/snippet}
		</CatalogList>
	{/if}
</section>

{#snippet monitoringControl(artist: ArtistSummary)}
	<ArtistMonitoringControl {artist} state={monitoringByArtistId[artist.id]} showError={false} />
{/snippet}

<style>
	.page-header {
		display: flex;
		align-items: flex-start;
		justify-content: space-between;
		gap: 1.5rem;
		margin-bottom: 2rem;
	}

	.list-header {
		display: grid;
		grid-template-columns: minmax(0, 1fr) 6.5rem 9.5rem;
		column-gap: 1rem;
		align-items: center;
		min-height: 2.75rem;
		padding: 0.5rem 1.25rem;
		border-bottom: 1px solid var(--border);
		background: color-mix(in oklch, var(--muted), var(--card) 50%);
		color: var(--muted-foreground);
		font-size: 0.75rem;
		font-weight: 500;
	}

	.monitor-heading {
		text-align: center;
	}

	.count-heading {
		text-align: right;
	}

	.monitoring-error {
		margin-bottom: 1rem;
		color: var(--destructive);
		font-size: 0.875rem;
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
			flex-direction: column;
			gap: 1.25rem;
			margin-bottom: 1.5rem;
		}

		.list-header {
			display: none;
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
