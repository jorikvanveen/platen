<script lang="ts">
	import EmptyState from "$lib/components/EmptyState.svelte";
	import PageHeading from "$lib/components/PageHeading.svelte";
	import ReleaseRow from "$lib/components/ReleaseRow.svelte";
	import { API_URL } from "$lib/constants";
	import type { PageProps } from "./$types";

	let { data }: PageProps = $props();
	let downloadState: Record<string, "starting" | "accepted" | "error"> = $state({});

	async function download(id: string) {
		if (data.albums.find((album) => album.id === id)?.downloaded) return;

		delete downloadState[id];
		downloadState[id] = "starting";
		try {
			const response = await fetch(`${API_URL}/albums/${id}/download`, { method: "POST" });
			if (!response.ok) {
				downloadState[id] = "error";
				return;
			}
			downloadState[id] = "accepted";
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
				{#if downloadState[album.id] === "accepted"}
					<div class="active-download">
						<a href="/downloads">View downloads</a>
					</div>
				{:else}
					<button
						class:success={album.downloaded}
						class:error={downloadState[album.id] === "error"}
						disabled={album.downloaded || downloadState[album.id] === "starting"}
						onclick={() => download(album.id)}
					>
						{downloadState[album.id] === "starting" ? "Starting…" : album.downloaded ? "Downloaded" : downloadState[album.id] === "error" ? "Retry download" : "Download"}
					</button>
				{/if}
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

	.credit-list a:hover,
	.active-download a:hover {
		text-decoration: underline;
	}

	.active-download {
		display: grid;
		justify-items: end;
		gap: 0.2rem;
		white-space: nowrap;
	}

	.active-download a {
		color: #aaa7b6;
		font-size: 0.78rem;
	}
</style>
