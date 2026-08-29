<script lang="ts">
	import AddAlbumButton from "$lib/components/AddAlbumButton.svelte";
	import EmptyState from "$lib/components/EmptyState.svelte";
	import PageHeading from "$lib/components/PageHeading.svelte";
	import ReleaseRow from "$lib/components/ReleaseRow.svelte";
	import { groupAlbums } from "$lib/albumGroups";
	import type { PageProps } from "./$types";

	let { data }: PageProps = $props();
</script>

<a class="back-link" href={`/artist/${data.artist.id}`}>← {data.artist.name}</a>
<PageHeading
	title={`More releases by ${data.artist.name}`}
	description="All releases from this artist on Tidal."
/>

{#if data.albums.length === 0}
	<EmptyState message="No releases found." />
{:else}
	{#each groupAlbums(data.albums) as group}
		<section class="release-group">
			<h2>{group.label}</h2>
			<div class="release-list">
				{#each group.albums as album (album.id)}
					{#snippet metadata()}
						{#if album.release_date}<span>{album.release_date}</span>{/if}
						<span>{album.album_type}</span>
					{/snippet}
					{#snippet action()}
						<AddAlbumButton albumId={album.id} />
					{/snippet}
					<ReleaseRow title={album.title} {metadata} {action} />
				{/each}
			</div>
		</section>
	{/each}
{/if}

<style>
	.back-link {
		display: inline-block;
		margin-bottom: 1.4rem;
		color: #aaa7b6;
	}

	.release-group + .release-group {
		margin-top: 2rem;
	}

	.release-list {
		display: grid;
		gap: 0.65rem;
	}
</style>
