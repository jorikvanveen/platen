<script lang="ts">
	import { onMount } from "svelte";
	import { deleteAlbum, getAlbumDeletionPreview } from "$lib/albumDeletion";
	import type { Album } from "$lib/dto/Album";
	import type { AlbumDeletionPreview } from "$lib/dto/AlbumDeletionPreview";
	import type { AlbumDeletionResult } from "$lib/dto/AlbumDeletionResult";

	let { album, oncancel, ondeleted }: {
		album: Album;
		oncancel: () => void;
		ondeleted: (result: AlbumDeletionResult) => Promise<void>;
	} = $props();

	let dialog: HTMLDialogElement;
	let preview: AlbumDeletionPreview | null = $state(null);
	let deleteFiles = $state(false);
	let loading = $state(true);
	let deleting = $state(false);
	let error = $state("");
	let deleted: AlbumDeletionResult | null = $state(null);

	onMount(() => {
		dialog.showModal();
		void loadPreview();
	});

	async function loadPreview() {
		loading = true;
		error = "";
		try {
			preview = await getAlbumDeletionPreview(fetch, album.id);
		} catch (caught) {
			error = caught instanceof Error ? caught.message : "Could not load the Album deletion preview.";
		} finally {
			loading = false;
		}
	}

	async function confirmDeletion() {
		if (!preview || deleting || deleted) return;
		deleting = true;
		error = "";
		try {
			deleted = await deleteAlbum(fetch, album.id, deleteFiles);
		} catch (caught) {
			error = caught instanceof Error ? caught.message : "Could not delete the Album.";
		} finally {
			deleting = false;
		}
		if (deleted) await updatePage();
	}

	async function updatePage() {
		if (!deleted || deleting) return;
		deleting = true;
		error = "";
		try {
			await ondeleted(deleted);
		} catch {
			error = "The Album was deleted, but the page could not be updated. Retry updating the page.";
		} finally {
			deleting = false;
		}
	}
</script>

<dialog bind:this={dialog} aria-labelledby="album-deletion-title" oncancel={(event) => {
	event.preventDefault();
	if (!deleting) oncancel();
}}>
	<h2 id="album-deletion-title">Delete "{album.title}"?</h2>
	{#if loading}
		<p role="status">Loading deletion preview…</p>
	{:else if preview}
		{#if !preview.absolute_path}
			<p id="album-location-warning">File location unknown. Only the catalog entry can be deleted.</p>
		{/if}
		<label>
			<input type="checkbox" bind:checked={deleteFiles}
				disabled={!preview.absolute_path || deleting || deleted !== null}
				aria-describedby={!preview.absolute_path ? "album-location-warning" : deleteFiles ? "album-disk-warning" : undefined} />
			Also delete files from disk
		</label>
		{#if deleteFiles}
			{#if preview.absolute_path}
				<p class="location">{preview.absolute_path}</p>
			{/if}
			<p id="album-disk-warning" class="warning">Permanently deletes this directory and everything inside. No undo.</p>
		{/if}
	{/if}
	{#if error}
		<p class="error-message" role="alert">{error}</p>
		{#if preview && !deleted}
			<p>Retry, or uncheck file deletion to remove only the catalog entry.</p>
		{/if}
	{/if}
	<div class="actions">
		<button class="secondary" disabled={deleting} onclick={oncancel}>{deleted ? "Close" : "Cancel"}</button>
		{#if deleted}
			<button disabled={deleting} onclick={updatePage}>{deleting ? "Updating…" : "Retry page update"}</button>
		{:else if preview}
			<button class="error" disabled={deleting} onclick={confirmDeletion}>
				{deleting ? "Deleting…" : deleteFiles ? "Delete Album and files" : "Delete Album"}
			</button>
		{:else if !loading}
			<button onclick={loadPreview}>Retry preview</button>
		{/if}
	</div>
</dialog>

<style>
	dialog {
		width: min(36rem, calc(100% - 2rem));
		max-height: calc(100% - 2rem);
		overflow-y: auto;
		border: 1px solid #302f38;
		border-radius: 0.8rem;
		padding: 1.5rem;
		color: #ecebf3;
		background: #19181e;
	}

	dialog::backdrop {
		background: #000a;
	}

	h2 {
		font-size: 1.3rem;
		overflow-wrap: anywhere;
	}

	p {
		line-height: 1.5;
	}

	.location {
		font-family: monospace;
		overflow-wrap: anywhere;
		color: #c9c6ff;
	}

	label {
		display: flex;
		align-items: baseline;
		gap: 0.6rem;
		margin-bottom: 1rem;
	}

	.warning,
	.error-message {
		color: #f0aaaa;
	}

	.actions {
		display: flex;
		flex-wrap: wrap;
		justify-content: flex-end;
		gap: 0.65rem;
	}

	.secondary {
		border-color: #55525f;
		background: #302f38;
	}
</style>
