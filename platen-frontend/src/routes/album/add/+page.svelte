<script lang="ts">
	import AddAlbumButton from "$lib/components/AddAlbumButton.svelte";
	import EmptyState from "$lib/components/EmptyState.svelte";
	import PageHeading from "$lib/components/PageHeading.svelte";
	import ReleaseRow from "$lib/components/ReleaseRow.svelte";
	import SearchForm from "$lib/components/SearchForm.svelte";
	import type { TidalAlbumSearchHit } from "$lib/dto/TidalAlbumSearchHit";
	import { navigateToSearch } from "$lib/searchNavigation";

	let { data }: { data: { query: string; albums: TidalAlbumSearchHit[] | null } } = $props();
	// svelte-ignore state_referenced_locally
	let query = $state(data.query);
	let loading = $state(false);

	async function onSearch() {
	    loading = true
		try {
	      await navigateToSearch("/album/add", query)
		} finally {
          loading = false
		}
	}
</script>

<PageHeading
	title="Add an album"
	description="Search Tidal by album title. Adding a release also adds every credited artist."
/>

<SearchForm {loading} bind:query ariaLabel="Album title" placeholder="Album title" onsearch={onSearch} />

{#if data.albums !== null}
	{#if data.albums.length === 0}
		<EmptyState message={`No albums matched "${data.query}".`} />
	{:else}
		<div class="results">
			{#each data.albums as album (album.id)}
				{#snippet metadata()}
					<span>{album.artists.map((artist) => artist.name).join(", ") || "Unknown artist"}</span>
					{#if album.release_date}<span>{album.release_date}</span>{/if}
					<span>{album.album_type}</span>
				{/snippet}
				{#snippet action()}
					<AddAlbumButton albumId={album.id} />
				{/snippet}
				<ReleaseRow title={album.title} {metadata} {action} />
			{/each}
		</div>
	{/if}
{/if}

<style>
	.results {
		display: grid;
		gap: 0.65rem;
	}
</style>
