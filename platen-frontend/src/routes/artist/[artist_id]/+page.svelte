<script lang="ts">
	import EmptyState from "$lib/components/EmptyState.svelte";
	import PageHeading from "$lib/components/PageHeading.svelte";
	import ReleaseRow from "$lib/components/ReleaseRow.svelte";
	import { API_URL } from "$lib/constants";
	import type { PageProps } from "./$types";

	let { data }: PageProps = $props();
	let downloadState: Record<string, "downloading" | "done" | "error"> = $state({});

	async function download(id: string) {
		downloadState[id] = "downloading";
		try {
			const response = await fetch(`${API_URL}/albums/${id}/download`, { method: "POST" });
			downloadState[id] = response.ok ? "done" : "error";
		} catch {
			downloadState[id] = "error";
		}
	}
</script>

<a class="back-link" href="/">←  All artists</a>
<PageHeading
	title={data.artist.name}
	description="Every release in your library that credits this artist."
	actions={[
		{ href: `/artist/${data.artist.id}/releases`, label: "Find more releases" },
	]}
/>

{#if data.albums.length === 0}
	<EmptyState message="No credited releases." />
{:else}
	<div class="release-list">
		{#each data.albums as album (album.id)}
			{#snippet metadata()}
				<span class="credit-list">
					{#each album.artists as artist, index (artist.id)}
						{#if index > 0}, {/if}<a href={`/artist/${artist.id}`}>{artist.name}</a>
					{/each}
				</span>
				<span>{album.release_year}</span>
				{#if album.album_type}<span>{album.album_type}</span>{/if}
			{/snippet}
			{#snippet action()}
				<button
					class:success={downloadState[album.id] === "done"}
					class:error={downloadState[album.id] === "error"}
					disabled={downloadState[album.id] === "downloading" || downloadState[album.id] === "done"}
					onclick={() => download(album.id)}
				>
					{downloadState[album.id] === "downloading" ? "Downloading…" : downloadState[album.id] === "done" ? "Downloaded" : downloadState[album.id] === "error" ? "Retry download" : "Download"}
				</button>
			{/snippet}
			<ReleaseRow title={album.title} {metadata} {action} />
		{/each}
	</div>
	{/if}

<style>
	.back-link {
		display: inline-block;
		margin-bottom: 1.4rem;
		color: #aaa7b6;
	}

	.release-list {
		display: grid;
		gap: 0.65rem;
	}

	.credit-list a {
		color: #c9c6ff;
		text-decoration: none;
	}

	.credit-list a:hover {
		text-decoration: underline;
	}
</style>
