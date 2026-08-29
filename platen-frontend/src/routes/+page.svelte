<script lang="ts">
	import EmptyState from "$lib/components/EmptyState.svelte";
	import PageHeading from "$lib/components/PageHeading.svelte";
	import type { PageProps } from "./$types";

	let { data }: PageProps = $props();
</script>

<PageHeading
	title="Your artists"
	description="Artists appear here when you add one of their credited releases."
	actions={[{ href: "/album/add", label: "Add an album" }]}
/>

{#if data.artists.length === 0}
	<EmptyState message="Your library is empty. Start with an album." />
{:else}
	<ul class="artist-list">
		{#each data.artists as artist (artist.id)}
			<li>
				<a href={`/artist/${artist.id}`}><strong>{artist.name}</strong></a>
			</li>
		{/each}
	</ul>
{/if}

<style>
	.artist-list {
		margin: 0;
		padding: 0;
		border-block: 1px solid #302f38;
		list-style: none;
	}

	.artist-list li + li {
		border-top: 1px solid #302f38;
	}

	.artist-list a {
		display: block;
		padding: 1rem 0.25rem;
		text-decoration: none;
	}

	.artist-list a:hover {
		background: #19181e;
	}

</style>
