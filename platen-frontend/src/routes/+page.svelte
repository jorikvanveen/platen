<script lang="ts">
	import type { PageProps } from './$types';

	let { data }: PageProps = $props();
</script>

<h1>Artists</h1>

{#if data.artists.length === 0}
	<p>No artists in the catalog yet.</p>
{:else}
	<ul>
		{#each data.artists as artist (artist.id)}
			<li>{artist.name}</li>
		{/each}
	</ul>
{/if}
